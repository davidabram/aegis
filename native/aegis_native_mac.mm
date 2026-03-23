#import <Cocoa/Cocoa.h>
#import <objc/runtime.h>

#include "aegis_app.h"
#include "aegis_native_mac.h"
#include "include/aegis_cef_host.hpp"
#include "include/base/cef_logging.h"
#include "include/cef_browser.h"
#include "include/cef_application_mac.h"
#include "include/wrapper/cef_library_loader.h"

#include <fstream>
#include <cstdlib>
#include <cstdio>
#include <filesystem>
#include <stdexcept>
#include <string>
#include <vector>

@interface AegisApplication : NSApplication <CefAppProtocol> {
 @private
  BOOL handlingSendEvent_;
}
@end

@implementation AegisApplication
- (BOOL)isHandlingSendEvent {
  return handlingSendEvent_;
}

- (void)setHandlingSendEvent:(BOOL)handlingSendEvent {
  handlingSendEvent_ = handlingSendEvent;
}

- (void)sendEvent:(NSEvent*)event {
  CefScopedSendingEvent sendingEventScoper;
  [super sendEvent:event];
}
@end

@interface NSAlert (AegisSuppression)
- (NSModalResponse)aegis_runModal;
@end

@implementation NSAlert (AegisSuppression)
- (NSModalResponse)aegis_runModal {
  return NSModalResponseCancel;
}
@end

static void InstallModalAlertSuppression(void) {
  static dispatch_once_t once_token;
  dispatch_once(&once_token, ^{
    Method original = class_getInstanceMethod([NSAlert class], @selector(runModal));
    Method replacement = class_getInstanceMethod([NSAlert class], @selector(aegis_runModal));
    method_exchangeImplementations(original, replacement);
  });
}

static std::string ExtractJsonStringValue(const std::string& json,
                                          const std::string& key) {
  const auto key_pattern = "\"" + key + "\"";
  const auto key_pos = json.find(key_pattern);
  if (key_pos == std::string::npos) {
    return {};
  }
  const auto colon_pos = json.find(':', key_pos + key_pattern.size());
  if (colon_pos == std::string::npos) {
    return {};
  }
  const auto first_quote = json.find('"', colon_pos + 1);
  if (first_quote == std::string::npos) {
    return {};
  }

  std::string value;
  bool escaping = false;
  for (std::size_t index = first_quote + 1; index < json.size(); ++index) {
    const auto ch = json[index];
    if (escaping) {
      value.push_back(ch);
      escaping = false;
      continue;
    }
    if (ch == '\\') {
      escaping = true;
      continue;
    }
    if (ch == '"') {
      break;
    }
    value.push_back(ch);
  }
  return value;
}

@interface AegisBrowserChromeView : NSView
@end

@interface AegisBrowserWindowController : NSObject <NSWindowDelegate, NSTextFieldDelegate>
- (instancetype)initWithStatePointer:(void*)state;
- (void)navigateBack:(id)sender;
- (void)navigateForward:(id)sender;
- (void)reloadPage:(id)sender;
- (void)submitLocation:(id)sender;
- (void)updateAddress:(NSString*)address;
- (void)updateNavigationButtonsWithCanGoBack:(BOOL)canGoBack
                                 canGoForward:(BOOL)canGoForward
                                    isLoading:(BOOL)isLoading;
@end

namespace {

struct BrowserHostState {
  NSWindow* window = nil;
  AegisBrowserChromeView* chrome_view = nil;
  NSView* root_view = nil;
  NSView* toolbar_view = nil;
  NSView* web_container_view = nil;
  NSButton* back_button = nil;
  NSButton* forward_button = nil;
  NSButton* reload_button = nil;
  NSButton* tab_button = nil;
  NSTextField* address_field = nil;
  AegisBrowserWindowController* controller = nil;
  CefRefPtr<CefBrowser> browser;
  std::string current_title = "Aegis";
  std::string current_url;
  bool can_go_back = false;
  bool can_go_forward = false;
  bool is_loading = false;
  bool closing_from_browser = false;
};

BrowserHostState& HostState() {
  static BrowserHostState state;
  return state;
}

std::filesystem::path StandaloneSupportDir() {
  NSArray<NSURL*>* urls = [[NSFileManager defaultManager]
      URLsForDirectory:NSApplicationSupportDirectory
             inDomains:NSUserDomainMask];
  NSURL* base_url = urls.count > 0 ? urls[0] : nil;
  if (base_url == nil) {
    throw std::runtime_error("failed to resolve Application Support directory");
  }
  const std::string base_path([[base_url path] UTF8String]);
  return std::filesystem::path(base_path) / "aegis_native";
}

void EnsureHostWindow(const std::string& title, int width, int height) {
  auto& state = HostState();
  if (state.window != nil && state.web_container_view != nil) {
    return;
  }

  const NSRect frame = NSMakeRect(0, 0, width, height);
  state.window = [[NSWindow alloc]
      initWithContentRect:frame
                styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                           NSWindowStyleMaskMiniaturizable |
                           NSWindowStyleMaskResizable)
                  backing:NSBackingStoreBuffered
                    defer:NO];
  if (state.window == nil) {
    throw std::runtime_error("failed to create browser host window");
  }

  state.current_title = title;
  state.current_url = title;
  [state.window setTitlebarAppearsTransparent:YES];
  [state.window setTitleVisibility:NSWindowTitleHidden];
  [state.window setToolbarStyle:NSWindowToolbarStyleUnifiedCompact];
  [state.window setMovableByWindowBackground:YES];
  [state.window setReleasedWhenClosed:NO];
  [state.window setBackgroundColor:[NSColor colorWithCalibratedWhite:0.97 alpha:1.0]];
  [state.window center];

  state.root_view =
      [[NSView alloc] initWithFrame:[[state.window contentView] bounds]];
  [state.root_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  [state.window setContentView:state.root_view];

  const CGFloat toolbar_height = 76.0;
  state.toolbar_view = [[NSVisualEffectView alloc]
      initWithFrame:NSMakeRect(0, height - toolbar_height, width, toolbar_height)];
  [(NSVisualEffectView*)state.toolbar_view setMaterial:NSVisualEffectMaterialSidebar];
  [(NSVisualEffectView*)state.toolbar_view setBlendingMode:NSVisualEffectBlendingModeBehindWindow];
  [(NSVisualEffectView*)state.toolbar_view setState:NSVisualEffectStateActive];
  [state.toolbar_view setAutoresizingMask:NSViewWidthSizable | NSViewMinYMargin];
  [state.root_view addSubview:state.toolbar_view];

  NSView* separator = [[NSView alloc] initWithFrame:NSMakeRect(0, toolbar_height - 1, width, 1)];
  [separator setAutoresizingMask:NSViewWidthSizable | NSViewMinYMargin];
  [separator setWantsLayer:YES];
  separator.layer.backgroundColor = [[NSColor colorWithCalibratedWhite:0.82 alpha:1.0] CGColor];
  [state.toolbar_view addSubview:separator];

  state.tab_button = [NSButton buttonWithTitle:@"Aegis"
                                        target:nil
                                        action:nil];
  [state.tab_button setFrame:NSMakeRect(18, toolbar_height - 42, 180, 28)];
  [state.tab_button setBezelStyle:NSBezelStyleRounded];
  [state.tab_button setBordered:NO];
  [state.tab_button setWantsLayer:YES];
  state.tab_button.layer.backgroundColor =
      [[NSColor colorWithCalibratedWhite:1.0 alpha:0.72] CGColor];
  state.tab_button.layer.cornerRadius = 14.0;
  state.tab_button.font = [NSFont systemFontOfSize:13 weight:NSFontWeightSemibold];
  state.tab_button.alignment = NSTextAlignmentLeft;
  state.tab_button.imagePosition = NSImageLeft;
  [state.tab_button setContentTintColor:[NSColor colorWithCalibratedWhite:0.18 alpha:1.0]];
  [state.toolbar_view addSubview:state.tab_button];

  state.back_button = [NSButton buttonWithTitle:@"<"
                                         target:nil
                                         action:nil];
  state.forward_button = [NSButton buttonWithTitle:@">"
                                            target:nil
                                            action:nil];
  state.reload_button = [NSButton buttonWithTitle:@"Reload"
                                           target:nil
                                           action:nil];

  NSArray<NSButton*>* nav_buttons = @[ state.back_button, state.forward_button, state.reload_button ];
  NSArray<NSNumber*>* nav_x = @[ @18, @58, @100 ];
  NSArray<NSNumber*>* nav_widths = @[ @32, @32, @72 ];
  for (NSUInteger index = 0; index < nav_buttons.count; ++index) {
    NSButton* button = nav_buttons[index];
    [button setFrame:NSMakeRect(nav_x[index].doubleValue, 14, nav_widths[index].doubleValue, 34)];
    [button setBezelStyle:NSBezelStyleTexturedRounded];
    [button setFont:[NSFont systemFontOfSize:13 weight:NSFontWeightSemibold]];
    [state.toolbar_view addSubview:button];
  }

  state.address_field = [[NSTextField alloc]
      initWithFrame:NSMakeRect(184, 14, width - 202, 34)];
  [state.address_field setAutoresizingMask:NSViewWidthSizable];
  [state.address_field setBezeled:NO];
  [state.address_field setFocusRingType:NSFocusRingTypeNone];
  [state.address_field setBordered:NO];
  [state.address_field setDrawsBackground:YES];
  [state.address_field setBackgroundColor:[NSColor colorWithCalibratedWhite:1.0 alpha:0.9]];
  [state.address_field setTextColor:[NSColor colorWithCalibratedWhite:0.14 alpha:1.0]];
  [state.address_field setFont:[NSFont systemFontOfSize:14 weight:NSFontWeightMedium]];
  [state.address_field setPlaceholderString:@"Search or enter address"];
  [state.address_field setWantsLayer:YES];
  state.address_field.layer.cornerRadius = 17.0;
  [state.toolbar_view addSubview:state.address_field];

  state.web_container_view =
      [[NSView alloc] initWithFrame:NSMakeRect(0, 0, width, height - toolbar_height)];
  [state.web_container_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  [state.root_view addSubview:state.web_container_view];

  state.controller = [[AegisBrowserWindowController alloc] initWithStatePointer:&state];
  [state.window setDelegate:state.controller];
  [state.back_button setTarget:state.controller];
  [state.back_button setAction:@selector(navigateBack:)];
  [state.forward_button setTarget:state.controller];
  [state.forward_button setAction:@selector(navigateForward:)];
  [state.reload_button setTarget:state.controller];
  [state.reload_button setAction:@selector(reloadPage:)];
  [state.address_field setTarget:state.controller];
  [state.address_field setAction:@selector(submitLocation:)];
  [state.address_field setDelegate:state.controller];
  [state.controller updateAddress:[NSString stringWithUTF8String:title.c_str()]];
  [state.controller updateNavigationButtonsWithCanGoBack:NO
                                            canGoForward:NO
                                               isLoading:NO];
}

}  // namespace

@implementation AegisBrowserChromeView
- (BOOL)isOpaque {
  return YES;
}

- (void)drawRect:(NSRect)dirtyRect {
  [[NSColor colorWithCalibratedWhite:0.97 alpha:1.0] setFill];
  NSRectFill(dirtyRect);
}
@end

@implementation AegisBrowserWindowController {
  void* _state_ptr;
}

- (instancetype)initWithStatePointer:(void*)state {
  self = [super init];
  if (self) {
    _state_ptr = state;
  }
  return self;
}

- (BrowserHostState*)state {
  return static_cast<BrowserHostState*>(_state_ptr);
}

- (void)navigateBack:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (state->browser && state->browser->CanGoBack()) {
    state->browser->GoBack();
  }
}

- (void)navigateForward:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (state->browser && state->browser->CanGoForward()) {
    state->browser->GoForward();
  }
}

- (void)reloadPage:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (state->browser) {
    if (state->is_loading) {
      state->browser->StopLoad();
    } else {
      state->browser->Reload();
    }
  }
}

- (void)submitLocation:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (!state->browser) {
    return;
  }
  NSString* value = [[state->address_field stringValue]
      stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
  if (value.length == 0) {
    return;
  }
  if ([value rangeOfString:@"://"].location == NSNotFound) {
    value = [@"https://" stringByAppendingString:value];
  }
  state->browser->GetMainFrame()->LoadURL(std::string([value UTF8String]));
}

- (void)updateAddress:(NSString*)address {
  auto* state = [self state];
  if (address == nil || state->address_field == nil) {
    return;
  }
  if (![[state->address_field currentEditor] isKindOfClass:[NSTextView class]]) {
    [state->address_field setStringValue:address];
  }
}

- (void)updateNavigationButtonsWithCanGoBack:(BOOL)canGoBack
                                 canGoForward:(BOOL)canGoForward
                                    isLoading:(BOOL)isLoading {
  auto* state = [self state];
  [state->back_button setEnabled:canGoBack];
  [state->forward_button setEnabled:canGoForward];
  [state->reload_button setTitle:isLoading ? @"Stop" : @"Reload"];
}

- (BOOL)windowShouldClose:(NSWindow*)window {
  BrowserHostState& state = HostState();
  if (state.closing_from_browser) {
    return YES;
  }
  if (state.browser) {
    state.browser->GetHost()->CloseBrowser(true);
    return NO;
  }
  [NSApp terminate:nil];
  return YES;
}
@end

std::string AegisStandaloneRootCachePath() {
  const auto root = StandaloneSupportDir() / "cef";
  std::filesystem::create_directories(root);
  return root.string();
}

std::string AegisStandaloneCachePath() {
  const auto cache = StandaloneSupportDir() / "cef" / "default-profile";
  std::filesystem::create_directories(cache);
  return cache.string();
}

CefWindowHandle AegisCreateBrowserHostView(const std::string& title,
                                           int width,
                                           int height) {
  EnsureHostWindow(title, width, height);
  return CAST_NSVIEW_TO_CEF_WINDOW_HANDLE(HostState().web_container_view);
}

void AegisShowBrowserHostWindow() {
  auto& state = HostState();
  if (state.window == nil) {
    return;
  }
  [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
  [NSApp activateIgnoringOtherApps:YES];
  [state.window makeKeyAndOrderFront:nil];
}

void AegisSetBrowserHostTitle(const std::string& title) {
  auto& state = HostState();
  if (state.window == nil) {
    return;
  }
  state.current_title = title;
  NSString* value = [NSString stringWithUTF8String:title.c_str()];
  [state.window setTitle:value];
  [state.tab_button setTitle:value];
}

void AegisSetBrowserHostAddress(const std::string& url) {
  auto& state = HostState();
  state.current_url = url;
  if (state.controller == nil) {
    return;
  }
  NSString* value = [NSString stringWithUTF8String:url.c_str()];
  [state.controller updateAddress:value];
}

void AegisSetBrowserHostNavigationState(bool can_go_back,
                                        bool can_go_forward,
                                        bool is_loading) {
  auto& state = HostState();
  state.can_go_back = can_go_back;
  state.can_go_forward = can_go_forward;
  state.is_loading = is_loading;
  if (state.controller == nil) {
    return;
  }
  [state.controller updateNavigationButtonsWithCanGoBack:can_go_back
                                            canGoForward:can_go_forward
                                               isLoading:is_loading];
}

void AegisAttachBrowserToHostWindow(CefRefPtr<CefBrowser> browser) {
  auto& state = HostState();
  if (state.window == nil || state.web_container_view == nil || !browser) {
    return;
  }

  state.browser = browser;
  NSView* browser_view =
      CAST_CEF_WINDOW_HANDLE_TO_NSVIEW(browser->GetHost()->GetWindowHandle());
  if (browser_view == nil) {
    return;
  }

  [browser_view setFrame:[state.web_container_view bounds]];
  [browser_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  if ([browser_view superview] != state.web_container_view) {
    [browser_view removeFromSuperview];
    [state.web_container_view addSubview:browser_view];
  }
}

void AegisCloseBrowserHostWindow() {
  auto& state = HostState();
  state.browser = nullptr;
  if (state.window == nil) {
    return;
  }
  state.closing_from_browser = true;
  [state.window orderOut:nil];
  [state.window close];
  state.closing_from_browser = false;
  state.window = nil;
  state.chrome_view = nil;
  state.root_view = nil;
  state.toolbar_view = nil;
  state.web_container_view = nil;
  state.back_button = nil;
  state.forward_button = nil;
  state.reload_button = nil;
  state.tab_button = nil;
  state.address_field = nil;
  state.controller = nil;
  [NSApp terminate:nil];
}

int main(int argc, char* argv[]) {
  CefScopedLibraryLoader loader;
  if (!loader.LoadInMain()) {
    return 1;
  }

  CefMainArgs main_args(argc, argv);

  std::string config_path;
  std::string request_path;
  std::string response_path;
  std::string error_path;
  std::string debug_log_path_arg;
  std::string startup_url;
  bool headful_mode = false;
  int operation_value = 0;
  for (int i = 1; i < argc; ++i) {
    const std::string arg(argv[i]);
    if (arg == "--mode" && i + 1 < argc) {
      headful_mode = std::string(argv[++i]) == "headful";
      continue;
    }
    if (arg == "--aegis-config" && i + 1 < argc) {
      config_path = argv[++i];
    } else if (arg == "--aegis-request" && i + 1 < argc) {
      request_path = argv[++i];
    } else if (arg == "--aegis-response" && i + 1 < argc) {
      response_path = argv[++i];
    } else if (arg == "--aegis-error" && i + 1 < argc) {
      error_path = argv[++i];
    } else if (arg == "--aegis-debug-log" && i + 1 < argc) {
      debug_log_path_arg = argv[++i];
    } else if (arg == "--url" && i + 1 < argc) {
      startup_url = argv[++i];
    } else if (arg == "--aegis-op" && i + 1 < argc) {
      operation_value = std::stoi(argv[++i]);
    }
  }
  const bool embedded_command_mode = !config_path.empty() && !request_path.empty() &&
                                     !response_path.empty() && operation_value != 0;
  const auto response_dir_end = response_path.find_last_of('/');
  const auto debug_log_path = !debug_log_path_arg.empty()
                                  ? debug_log_path_arg
                                  : (embedded_command_mode
                                         ? ((response_dir_end == std::string::npos
                                                 ? std::string(".")
                                                 : response_path.substr(0, response_dir_end)) +
                                            "/debug.log")
                                         : std::string());
  auto append_debug = [&debug_log_path](const std::string& message) {
    if (debug_log_path.empty()) {
      return;
    }
    std::ofstream output(debug_log_path, std::ios::app);
    if (!output.is_open()) {
      return;
    }
    output << message << '\n';
  };

  @autoreleasepool {
    if (embedded_command_mode) {
      std::ofstream(debug_log_path, std::ios::trunc).close();
      unsetenv("AEGIS_DEBUG_LOG");
      setenv("AEGIS_DEBUG_LOG", debug_log_path.c_str(), 1);
      append_debug("main: embedded command mode");
    }
    [AegisApplication sharedApplication];
    if (!embedded_command_mode) {
      [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
      [NSApp finishLaunching];
    }

    CefSettings settings;
#if !defined(CEF_USE_SANDBOX)
    settings.no_sandbox = true;
#endif
    settings.windowless_rendering_enabled = true;
    settings.command_line_args_disabled = false;
    settings.external_message_pump = embedded_command_mode;

    if (!embedded_command_mode) {
      const auto root_cache_path = AegisStandaloneRootCachePath();
      const auto cache_path = AegisStandaloneCachePath();
      CefString(&settings.root_cache_path) = root_cache_path;
      CefString(&settings.cache_path) = cache_path;
    } else if (!config_path.empty()) {
      std::ifstream config_input(config_path, std::ios::binary);
      if (config_input.is_open()) {
        const std::string config_json((std::istreambuf_iterator<char>(config_input)),
                                      std::istreambuf_iterator<char>());
        const auto user_data_dir = ExtractJsonStringValue(config_json, "user_data_dir");
        if (!user_data_dir.empty()) {
          CefString(&settings.cache_path) = user_data_dir;
          CefString(&settings.root_cache_path) = user_data_dir;
          append_debug("main: configured cache path " + user_data_dir);
        }
      }
    }

    CefRefPtr<AegisApp> app(new AegisApp(!embedded_command_mode, startup_url));
    append_debug("main: before CefExecuteProcess");
    const int subprocess_exit_code = CefExecuteProcess(main_args, app.get(), nullptr);
    append_debug("main: after CefExecuteProcess=" + std::to_string(subprocess_exit_code));
    if (subprocess_exit_code >= 0) {
      return subprocess_exit_code;
    }

    append_debug("main: before CefInitialize");
    if (!CefInitialize(main_args, settings, app.get(), nullptr)) {
      append_debug("main: CefInitialize failed");
      return CefGetExitCode();
    }
    append_debug("main: after CefInitialize");

    if (embedded_command_mode && !headful_mode) {
      [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
      InstallModalAlertSuppression();
      append_debug("main: activation policy configured");
    } else if (!embedded_command_mode) {
      [NSApp activateIgnoringOtherApps:YES];
    }

    if (embedded_command_mode) {
      auto read_file = [](const std::string& path, std::vector<std::uint8_t>* out) -> bool {
        std::ifstream input(path, std::ios::binary);
        if (!input.is_open()) {
          return false;
        }
        *out = std::vector<std::uint8_t>((std::istreambuf_iterator<char>(input)),
                                         std::istreambuf_iterator<char>());
        return true;
      };
      auto write_file = [](const std::string& path, const std::string& value) -> bool {
        std::ofstream output(path, std::ios::binary | std::ios::trunc);
        if (!output.is_open()) {
          return false;
        }
        output.write(value.data(), static_cast<std::streamsize>(value.size()));
        return output.good();
      };
      auto write_bytes = [](const std::string& path, const std::vector<std::uint8_t>& bytes) -> bool {
        std::ofstream output(path, std::ios::binary | std::ios::trunc);
        if (!output.is_open()) {
          return false;
        }
        if (!bytes.empty()) {
          output.write(reinterpret_cast<const char*>(bytes.data()),
                       static_cast<std::streamsize>(bytes.size()));
        }
        return output.good();
      };

      int exit_code = 0;
      std::vector<std::uint8_t> config_bytes;
      std::vector<std::uint8_t> request_bytes;
      if (!read_file(config_path, &config_bytes)) {
        if (!error_path.empty()) {
          write_file(error_path, "failed to read config file");
        }
        exit_code = 3;
      } else if (!read_file(request_path, &request_bytes)) {
        if (!error_path.empty()) {
          write_file(error_path, "failed to read request file");
        }
        exit_code = 3;
      } else {
        append_debug("main: before embedded host operation");
        std::vector<std::uint8_t> response;
        std::string error;
        const bool ok = aegis::RunEmbeddedHostOperation(
            config_bytes,
            static_cast<aegis::EmbeddedHostOperation>(operation_value),
            request_bytes,
            &response,
            &error);
        append_debug(std::string("main: embedded host operation result=") +
                     (ok ? "ok" : "error"));
        if (!ok) {
          if (!error_path.empty()) {
            write_file(error_path, error);
          }
          exit_code = 2;
        } else if (!write_bytes(response_path, response)) {
          if (!error_path.empty()) {
            write_file(error_path, "failed to write response file");
          }
          exit_code = 3;
        }
      }

      append_debug("main: embedded command complete");
      std::fflush(nullptr);
      std::_Exit(exit_code);
    }

    CefRunMessageLoop();
    CefShutdown();
  }

  return 0;
}
