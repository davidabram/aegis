#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#import <objc/runtime.h>

#include "include/aegis_platform.h"
#include "include/cef_application_mac.h"

#include <stdexcept>
#include <string>

namespace {

std::filesystem::path DetectAppBundle(const std::filesystem::path& anchor) {
  for (auto current = anchor; !current.empty(); current = current.parent_path()) {
    if (current.extension() == ".app") {
      return current;
    }
    if (current == current.root_path()) {
      break;
    }
  }
  return {};
}

}  // namespace

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

int AegisPlatformRunMain(AegisPlatformMainEntry entry, int argc, char* argv[]) {
  @autoreleasepool {
    return entry(argc, argv);
  }
}

void AegisPlatformInitializeMainApplication(bool embedded_command_mode) {
  [AegisApplication sharedApplication];
  if (!embedded_command_mode) {
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    [NSApp finishLaunching];
  }
}

void AegisPlatformConfigureActivation(bool embedded_command_mode, bool headful_mode) {
  if (embedded_command_mode && !headful_mode) {
    [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
    AegisInstallModalAlertSuppression();
  } else if (!embedded_command_mode) {
    [NSApp activateIgnoringOtherApps:YES];
  }
}

void AegisInstallModalAlertSuppression() {
  static dispatch_once_t once_token;
  dispatch_once(&once_token, ^{
    Method original = class_getInstanceMethod([NSAlert class], @selector(runModal));
    Method replacement = class_getInstanceMethod([NSAlert class], @selector(aegis_runModal));
    method_exchangeImplementations(original, replacement);
  });
}

void AegisConfigureCefSettings(const AegisCefBootstrapOptions& options,
                               CefSettings* settings) {
  if (settings == nullptr) {
    return;
  }
#if !defined(CEF_USE_SANDBOX)
  settings->no_sandbox = true;
#endif
  settings->windowless_rendering_enabled = true;
  settings->command_line_args_disabled = false;
  settings->external_message_pump = options.external_message_pump;

  if (!options.browser_subprocess_path.empty()) {
    CefString(&settings->browser_subprocess_path) = options.browser_subprocess_path;
  }
  if (!options.framework_dir_path.empty()) {
    CefString(&settings->framework_dir_path) = options.framework_dir_path;
  }
  if (!options.main_bundle_path.empty()) {
    CefString(&settings->main_bundle_path) = options.main_bundle_path;
  }
  if (!options.resources_dir_path.empty()) {
    CefString(&settings->resources_dir_path) = options.resources_dir_path;
  }
  if (!options.locales_dir_path.empty()) {
    CefString(&settings->locales_dir_path) = options.locales_dir_path;
  }
  if (!options.root_cache_path.empty()) {
    CefString(&settings->root_cache_path) = options.root_cache_path;
  }
  if (!options.cache_path.empty()) {
    CefString(&settings->cache_path) = options.cache_path;
  }
}

bool AegisExecuteProcessAndInitialize(const CefMainArgs& main_args,
                                      const AegisCefBootstrapOptions& options,
                                      CefRefPtr<CefApp> app,
                                      int* subprocess_exit_code,
                                      std::string* error) {
  if (options.headless) {
    AegisInstallModalAlertSuppression();
  }
  if (options.initialize_browser_host_application) {
    AegisInitializeBrowserHostApplication();
  }

  CefSettings settings;
  AegisConfigureCefSettings(options, &settings);

  const int execute_process_result = CefExecuteProcess(main_args, app.get(), nullptr);
  if (subprocess_exit_code != nullptr) {
    *subprocess_exit_code = execute_process_result;
  }
  if (execute_process_result >= 0) {
    return false;
  }
  if (!CefInitialize(main_args, settings, app.get(), nullptr)) {
    if (error != nullptr) {
      *error = "CefInitialize failed";
    }
    return false;
  }
  return true;
}

AegisPlatformPaths AegisResolvePlatformPaths(
    const std::filesystem::path& library_dir) {
  auto app_bundle = DetectAppBundle(library_dir);
  const auto app_root =
      !app_bundle.empty() ? app_bundle.parent_path().parent_path() : library_dir;
  if (app_bundle.empty()) {
    app_bundle = app_root / "aegis_native.app";
  }
  AegisPlatformPaths paths{
      .library_dir = library_dir,
      .app_root = app_root,
      .main_executable = app_bundle / "Contents" / "MacOS" / "aegis_native",
      .helper_executable = app_bundle / "Contents" / "Frameworks" /
                           "aegis_native Helper.app" / "Contents" / "MacOS" /
                           "aegis_native Helper",
      .cef_library = app_bundle / "Contents" / "Frameworks" /
                     "Chromium Embedded Framework.framework" /
                     "Chromium Embedded Framework",
      .framework_dir = app_bundle / "Contents" / "Frameworks" /
                       "Chromium Embedded Framework.framework",
      .resources_dir = app_bundle / "Contents" / "Frameworks" /
                       "Chromium Embedded Framework.framework" / "Resources",
      .locales_dir = app_bundle / "Contents" / "Frameworks" /
                     "Chromium Embedded Framework.framework" / "Resources" /
                     "locales",
      .main_bundle_path = app_bundle,
  };

  if (!std::filesystem::exists(paths.main_bundle_path)) {
    throw std::runtime_error("aegis_native.app is missing; build the native app bundle first");
  }
  if (!std::filesystem::exists(paths.helper_executable)) {
    throw std::runtime_error("aegis_native Helper is missing; build the native helper bundle first");
  }
  if (!std::filesystem::exists(paths.framework_dir)) {
    throw std::runtime_error("Chromium Embedded Framework.framework is missing from the app bundle");
  }

  return paths;
}

bool AegisUseExternalBrowserHostWindow() { return true; }

#include "aegis_browser_host_mac.inc"
