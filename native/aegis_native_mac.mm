#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
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

// ─── CefAppProtocol ──────────────────────────────────────────────────────────

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

// ─── Modal alert suppression ─────────────────────────────────────────────────

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

// ─── JSON helper ─────────────────────────────────────────────────────────────

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

// ═══════════════════════════════════════════════════════════════════════════════
// ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════════════

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
