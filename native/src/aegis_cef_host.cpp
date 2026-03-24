#include "aegis_cef_host.hpp"

#include <dlfcn.h>

#include <chrono>
#include <cctype>
#include <condition_variable>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <fstream>
#include <functional>
#include <cstdlib>
#include <cerrno>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <pthread.h>
#include <signal.h>
#include <stdexcept>
#include <string>
#include <thread>
#include <unistd.h>
#include <utility>
#include <vector>

#include "../aegis_app.h"
#include "../aegis_client.h"
#include "../aegis_messages.h"
#include "../aegis_native_mac.h"
#include "include/base/cef_bind.h"
#include "include/cef_app.h"
#include "include/cef_browser.h"
#include "include/cef_cookie.h"
#include "include/cef_parser.h"
#include "include/cef_request_context.h"
#include "include/cef_waitable_event.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_window.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"
#include "include/wrapper/cef_library_loader.h"

namespace aegis {
namespace {

constexpr auto kStartupTimeout = std::chrono::seconds(30);
constexpr auto kRendererTimeout = std::chrono::seconds(30);
constexpr auto kShutdownTimeout = std::chrono::seconds(2);
constexpr auto kPumpInterval = std::chrono::milliseconds(10);
constexpr char kBootstrapUrl[] =
    "data:text/html,%3C!doctype%20html%3E%3Chtml%3E%3Chead%3E%3Cmeta%20charset%3D%22utf-8%22%3E%3C%2Fhead%3E%3Cbody%3E%3C%2Fbody%3E%3C%2Fhtml%3E";

void AppendDebugLog(const std::string& message) {
  const char* path = std::getenv("AEGIS_DEBUG_LOG");
  if (path == nullptr || *path == '\0') {
    return;
  }
  std::ofstream output(path, std::ios::app);
  if (!output.is_open()) {
    return;
  }
  output << message << '\n';
}

std::vector<std::uint8_t> CopyInput(const std::uint8_t* input_ptr, std::size_t input_len) {
  if (input_ptr == nullptr || input_len == 0) {
    return {};
  }
  return {input_ptr, input_ptr + input_len};
}

void WriteOutput(std::vector<std::uint8_t> bytes, AegisHostBuffer* output) {
  if (output == nullptr) {
    throw std::runtime_error("output buffer is null");
  }

  if (bytes.empty()) {
    output->ptr = nullptr;
    output->len = 0;
    return;
  }

  auto* heap = new std::uint8_t[bytes.size()];
  std::copy(bytes.begin(), bytes.end(), heap);
  output->ptr = heap;
  output->len = bytes.size();
}

CefRefPtr<CefDictionaryValue> RequireDictionary(CefRefPtr<CefValue> value,
                                                const char* message) {
  if (!value.get() || value->GetType() != VTYPE_DICTIONARY) {
    throw std::runtime_error(message);
  }
  return value->GetDictionary()->Copy(false);
}

CefRefPtr<CefValue> ParseJsonValue(const std::string& json, const char* message) {
  auto value = CefParseJSON(json, JSON_PARSER_RFC);
  if (!value.get()) {
    throw std::runtime_error(message);
  }
  return value;
}

std::string WriteJson(CefRefPtr<CefValue> value) {
  return CefWriteJSON(value, JSON_WRITER_DEFAULT).ToString();
}

std::string WriteJson(CefRefPtr<CefDictionaryValue> value) {
  auto wrapped = CefValue::Create();
  wrapped->SetDictionary(value);
  return WriteJson(wrapped);
}

std::filesystem::path LibraryDirectory() {
  Dl_info info{};
  if (dladdr(reinterpret_cast<void*>(&aegis_get_function_table), &info) == 0 ||
      info.dli_fname == nullptr) {
    throw std::runtime_error("failed to resolve host library path");
  }
  return std::filesystem::path(info.dli_fname).parent_path();
}

std::filesystem::path HostSupportDir() {
  const char* home = std::getenv("HOME");
  if (home == nullptr || *home == '\0') {
    throw std::runtime_error("HOME is not set");
  }
  return std::filesystem::path(home) / "Library" / "Application Support" / "aegis_native";
}

std::filesystem::path HostRootCacheDir() {
  return HostSupportDir() / "agent-runtime";
}

std::filesystem::path HostInstancesDir() {
  return HostRootCacheDir() / "instances";
}

struct HostRuntimePaths {
  std::filesystem::path root_cache_dir;
  std::filesystem::path profile_dir;
};

std::string HostRuntimeInstanceId() {
  const auto now = std::chrono::system_clock::now().time_since_epoch();
  const auto nanos =
      std::chrono::duration_cast<std::chrono::nanoseconds>(now).count();
  return std::to_string(static_cast<long long>(::getpid())) + "-" +
         std::to_string(static_cast<long long>(nanos));
}

HostRuntimePaths CreateHostRuntimePaths() {
  const auto instance_root = HostInstancesDir() / HostRuntimeInstanceId();
  const auto profile_dir = instance_root / "profile";
  std::filesystem::create_directories(profile_dir);
  return {
      .root_cache_dir = instance_root,
      .profile_dir = profile_dir,
  };
}

bool ProcessExists(pid_t pid) {
  if (pid <= 0) {
    return false;
  }
  if (::kill(pid, 0) == 0) {
    return true;
  }
  return errno == EPERM;
}

std::optional<pid_t> ParseInstancePid(const std::filesystem::path& path) {
  const auto name = path.filename().string();
  const auto dash = name.find('-');
  const auto pid_text = dash == std::string::npos ? name : name.substr(0, dash);
  if (pid_text.empty()) {
    return std::nullopt;
  }
  for (const char ch : pid_text) {
    if (!std::isdigit(static_cast<unsigned char>(ch))) {
      return std::nullopt;
    }
  }
  try {
    return static_cast<pid_t>(std::stoll(pid_text));
  } catch (...) {
    return std::nullopt;
  }
}

void RemoveTreeIfExists(const std::filesystem::path& path) {
  std::error_code error;
  std::filesystem::remove_all(path, error);
}

void CleanupLegacyRuntimeRoot() {
  const auto root = HostRootCacheDir();
  std::error_code error;
  std::filesystem::create_directories(root, error);
  if (error) {
    return;
  }

  for (const auto& entry : std::filesystem::directory_iterator(root, error)) {
    if (error) {
      return;
    }
    if (entry.path().filename() == "instances") {
      continue;
    }
    RemoveTreeIfExists(entry.path());
  }
}

void CleanupStaleRuntimeInstances() {
  const auto instances_dir = HostInstancesDir();
  std::error_code error;
  std::filesystem::create_directories(instances_dir, error);
  if (error) {
    return;
  }

  for (const auto& entry : std::filesystem::directory_iterator(instances_dir, error)) {
    if (error) {
      return;
    }
    const auto pid = ParseInstancePid(entry.path());
    if (!pid.has_value() || ProcessExists(*pid)) {
      continue;
    }
    RemoveTreeIfExists(entry.path());
  }
}

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

struct HostPaths {
  std::filesystem::path library_dir;
  std::filesystem::path app_bundle;
  std::filesystem::path main_executable;
  std::filesystem::path helper_executable;
  std::filesystem::path framework_dir;
  std::filesystem::path resources_dir;
  std::filesystem::path locales_dir;
};

HostPaths ResolveHostPaths() {
  const auto anchor_dir = LibraryDirectory();
  auto app_bundle = DetectAppBundle(anchor_dir);
  const auto library_dir =
      !app_bundle.empty() ? app_bundle.parent_path().parent_path() : anchor_dir;
  if (app_bundle.empty()) {
    app_bundle = library_dir / "aegis_native.app";
  }
  HostPaths paths{
      .library_dir = library_dir,
      .app_bundle = app_bundle,
      .main_executable = app_bundle / "Contents" / "MacOS" / "aegis_native",
      .helper_executable = app_bundle / "Contents" / "Frameworks" /
                           "aegis_native Helper.app" / "Contents" / "MacOS" / "aegis_native Helper",
      .framework_dir = app_bundle / "Contents" / "Frameworks" /
                       "Chromium Embedded Framework.framework",
      .resources_dir = app_bundle / "Contents" / "Frameworks" /
                       "Chromium Embedded Framework.framework" / "Resources",
      .locales_dir = app_bundle / "Contents" / "Frameworks" /
                     "Chromium Embedded Framework.framework" / "Resources" / "locales",
  };

  if (!std::filesystem::exists(paths.app_bundle)) {
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

struct BrowserOptions {
  bool headless = true;
  std::string start_url = kBootstrapUrl;
};

BrowserOptions ParseBrowserOptions(const std::vector<std::uint8_t>& bytes) {
  BrowserOptions options;
  if (bytes.empty()) {
    return options;
  }

  const auto json = std::string(bytes.begin(), bytes.end());

  auto skip_whitespace = [&](std::size_t* index) {
    while (*index < json.size() &&
           std::isspace(static_cast<unsigned char>(json[*index])) != 0) {
      ++(*index);
    }
  };

  std::function<std::string(std::size_t*)> parse_string = [&](std::size_t* index) {
    if (*index >= json.size() || json[*index] != '"') {
      throw std::runtime_error("browser config is not valid json");
    }
    ++(*index);
    std::string value;
    while (*index < json.size()) {
      const char ch = json[*index];
      ++(*index);
      if (ch == '"') {
        return value;
      }
      if (ch != '\\') {
        value.push_back(ch);
        continue;
      }
      if (*index >= json.size()) {
        throw std::runtime_error("browser config is not valid json");
      }
      const char escaped = json[*index];
      ++(*index);
      switch (escaped) {
        case '"':
        case '\\':
        case '/':
          value.push_back(escaped);
          break;
        case 'b':
          value.push_back('\b');
          break;
        case 'f':
          value.push_back('\f');
          break;
        case 'n':
          value.push_back('\n');
          break;
        case 'r':
          value.push_back('\r');
          break;
        case 't':
          value.push_back('\t');
          break;
        case 'u':
          throw std::runtime_error("unicode escapes are not supported in browser config");
        default:
          throw std::runtime_error("browser config is not valid json");
      }
    }
    throw std::runtime_error("browser config is not valid json");
  };

  std::size_t index = 0;
  skip_whitespace(&index);
  if (index >= json.size() || json[index] != '{') {
    throw std::runtime_error("browser config must be a dictionary");
  }
  ++index;

  while (true) {
    skip_whitespace(&index);
    if (index >= json.size()) {
      throw std::runtime_error("browser config is not valid json");
    }
    if (json[index] == '}') {
      ++index;
      break;
    }

    const auto key = parse_string(&index);
    skip_whitespace(&index);
    if (index >= json.size() || json[index] != ':') {
      throw std::runtime_error("browser config is not valid json");
    }
    ++index;
    skip_whitespace(&index);
    if (index >= json.size()) {
      throw std::runtime_error("browser config is not valid json");
    }

    if (json[index] == '"') {
      const auto value = parse_string(&index);
      if (key == "mode") {
        options.headless = value != "headful";
      } else if (key == "start_url") {
        if (!value.empty()) {
          options.start_url = value;
        }
      }
    } else if (json.compare(index, 4, "null") == 0) {
      index += 4;
    } else {
      throw std::runtime_error("browser config contains unsupported value type");
    }

    skip_whitespace(&index);
    if (index >= json.size()) {
      throw std::runtime_error("browser config is not valid json");
    }
    if (json[index] == ',') {
      ++index;
      continue;
    }
    if (json[index] == '}') {
      ++index;
      break;
    }
    throw std::runtime_error("browser config is not valid json");
  }

  return options;
}

std::string CookieUrl(const CefRefPtr<CefDictionaryValue>& cookie) {
  auto domain = cookie->GetString("domain").ToString();
  while (!domain.empty() && domain.front() == '.') {
    domain.erase(domain.begin());
  }
  const auto path = cookie->HasKey("path") ? cookie->GetString("path").ToString() : std::string("/");
  const auto secure = cookie->HasKey("secure") && cookie->GetBool("secure");
  return (secure ? "https://" : "http://") + domain + (path.empty() ? "/" : path);
}

class CompletionSignal : public CefCompletionCallback {
 public:
  explicit CompletionSignal(CefRefPtr<CefWaitableEvent> event) : event_(event) {}

  void OnComplete() override { event_->Signal(); }

 private:
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(CompletionSignal);
};

class SetCookieSignal : public CefSetCookieCallback {
 public:
  explicit SetCookieSignal(CefRefPtr<CefWaitableEvent> event) : event_(event) {}

  void OnComplete(bool) override { event_->Signal(); }

 private:
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(SetCookieSignal);
};

class DeleteCookieSignal : public CefDeleteCookiesCallback {
 public:
  explicit DeleteCookieSignal(CefRefPtr<CefWaitableEvent> event) : event_(event) {}

  void OnComplete(int) override { event_->Signal(); }

 private:
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(DeleteCookieSignal);
};

class CookieCollector : public CefCookieVisitor {
 public:
  CookieCollector(std::vector<CefCookie>* cookies, CefRefPtr<CefWaitableEvent> event)
      : cookies_(cookies), event_(event) {}

  ~CookieCollector() override { event_->Signal(); }

  bool Visit(const CefCookie& cookie, int, int, bool&) override {
    cookies_->push_back(cookie);
    return true;
  }

 private:
  std::vector<CefCookie>* cookies_;
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(CookieCollector);
};

class HostWindowDelegate : public CefWindowDelegate {
 public:
  explicit HostWindowDelegate(CefRefPtr<CefBrowserView> browser_view)
      : browser_view_(browser_view) {}

  void OnWindowCreated(CefRefPtr<CefWindow> window) override {
    window->AddChildView(browser_view_);
    window->Show();
  }

  bool CanClose(CefRefPtr<CefWindow>) override { return true; }

 private:
  CefRefPtr<CefBrowserView> browser_view_;

  IMPLEMENT_REFCOUNTING(HostWindowDelegate);
};

class UiClosureTask : public CefTask {
 public:
  UiClosureTask(std::function<void()> work,
                std::shared_ptr<std::exception_ptr> error,
                CefRefPtr<CefWaitableEvent> done)
      : work_(std::move(work)), error_(std::move(error)), done_(done) {}

  void Execute() override {
    try {
      work_();
    } catch (...) {
      *error_ = std::current_exception();
    }
    done_->Signal();
  }

 private:
  std::function<void()> work_;
  std::shared_ptr<std::exception_ptr> error_;
  CefRefPtr<CefWaitableEvent> done_;

  IMPLEMENT_REFCOUNTING(UiClosureTask);
};

struct RendererReply {
  bool ok = false;
  std::string body;
};

class AegisCefHost;

class AegisHostClient final : public AegisClient {
 public:
  AegisHostClient(bool headless,
                  ::AegisClientDelegate* delegate,
                  AegisCefHost* host)
      : AegisClient(headless, delegate), host_(host) {}

  bool OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                CefProcessId source_process,
                                CefRefPtr<CefProcessMessage> message) override;

 private:
  AegisCefHost* host_;

  IMPLEMENT_REFCOUNTING(AegisHostClient);
};

class AegisCefHost final : public CefHost, public ::AegisClientDelegate {
 public:
  explicit AegisCefHost(BrowserOptions options, bool manage_cef_lifecycle = true)
      : options_(std::move(options)),
        paths_(ResolveHostPaths()),
        runtime_paths_(CreateHostRuntimePaths()),
        owner_thread_id_(std::this_thread::get_id()),
        manage_cef_lifecycle_(manage_cef_lifecycle) {
    if (pthread_main_np() == 0) {
      throw std::runtime_error("aegis CEF host must be created on the process main thread");
    }
    AppendDebugLog("host: constructed");
    CleanupLegacyRuntimeRoot();
    CleanupStaleRuntimeInstances();
    if (manage_cef_lifecycle_) {
      Start();
    } else {
      AttachToInitializedCef();
    }
  }

  ~AegisCefHost() override { Shutdown(); }

  void WaitForReady() {
    AppendDebugLog("host: wait_for_ready enter");
    std::unique_lock lock(mutex_);
    if (!cv_.wait_for(lock, kStartupTimeout, [this] {
          return startup_complete_ || !startup_error_.empty();
        })) {
      throw std::runtime_error("timed out waiting for CEF startup");
    }
    if (!startup_error_.empty()) {
      throw std::runtime_error(startup_error_);
    }
    AppendDebugLog("host: wait_for_ready complete");
  }

  std::vector<std::uint8_t> InstallRuntime(const std::vector<std::uint8_t>& request) override {
    auto payload = DecodeEnvelope(MessageKind::InstallRuntime, request);
    if (payload->GetType() != VTYPE_STRING) {
      throw std::runtime_error("install runtime payload must be a string");
    }
    runtime_script_ = payload->GetString().ToString();
    EnsureRuntimeInstalled();
    return {};
  }

  std::vector<std::uint8_t> EvalJs(const std::vector<std::uint8_t>& request) override {
    auto payload = RequireDictionary(
        DecodeEnvelope(MessageKind::EvalJs, request), "eval request must be a dictionary");
    const auto result = InvokeRenderer(aegis::kOpEvalJs, payload->GetString("script").ToString());

    auto response = CefDictionaryValue::Create();
    auto bytes = CefListValue::Create();
    for (std::size_t index = 0; index < result.size(); ++index) {
      bytes->SetInt(static_cast<int>(index),
                    static_cast<unsigned char>(result[static_cast<std::size_t>(index)]));
    }
    response->SetList("value", bytes);
    return EncodeEnvelope(MessageKind::EvalJs, response);
  }

  std::vector<std::uint8_t> SendBatch(const std::vector<std::uint8_t>& request) override {
    EnsureRuntimeInstalled();
    auto payload =
        RequireDictionary(DecodeEnvelope(MessageKind::SendBatch, request),
                          "batch request must be a dictionary");
    const auto body = WriteJson(payload);
    const auto response = InvokeRenderer(aegis::kOpSendBatch, body);
    return EncodeJsonEnvelope(MessageKind::SendBatch, response);
  }

  std::vector<std::uint8_t> SnapshotDom(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    EnsureRuntimeInstalled();
    return EncodeJsonEnvelope(MessageKind::SnapshotDom,
                              InvokeRenderer(aegis::kOpSnapshotDom, "{}"));
  }

  std::vector<std::uint8_t> InjectSession(const std::vector<std::uint8_t>& request) override {
    auto payload = RequireDictionary(
        DecodeEnvelope(MessageKind::InjectSession, request),
        "session request must be a dictionary");

    ReplaceNetworkOverrides(payload);
    ReplaceCookies(payload);
    EnsureRuntimeInstalled();
    InvokeRenderer(aegis::kOpInjectStorage, WriteJson(payload));
    return {};
  }

  std::vector<std::uint8_t> SnapshotSession(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    EnsureRuntimeInstalled();

    auto storage = RequireDictionary(
        ParseJsonValue(InvokeRenderer(aegis::kOpSnapshotStorage, "{}"),
                       "storage snapshot is not valid json"),
        "storage snapshot must be a dictionary");
    storage->SetList("cookies", SnapshotCookies());
    storage->SetList("network_overrides", SnapshotNetworkOverrides());
    return EncodeEnvelope(MessageKind::SnapshotSession, storage);
  }

  std::vector<std::uint8_t> DrainEvents(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    EnsureRuntimeInstalled();

    const auto renderer_response = InvokeRenderer(aegis::kOpDrainEvents, "{}");
    AppendDebugLog("host: drain_events renderer_response bytes=" +
                   std::to_string(renderer_response.size()));
    auto response = RequireDictionary(
        ParseJsonValue(renderer_response,
                       "drain events response is not valid json"),
        "drain events response must be a dictionary");
    AppendDebugLog("host: drain_events parsed_response");
    auto existing_events =
        response->HasKey("events") ? response->GetList("events") : CefListValue::Create();
    AppendDebugLog("host: drain_events existing_events=" +
                   std::to_string(existing_events ? existing_events->GetSize() : 0));
    auto events = CefListValue::Create();
    if (existing_events.get()) {
      for (size_t index = 0; index < existing_events->GetSize(); ++index) {
        auto value = existing_events->GetValue(static_cast<int>(index));
        if (value.get()) {
          events->SetValue(static_cast<int>(index), value->Copy());
        }
      }
    }

    auto local_events = DrainLocalEvents();
    AppendDebugLog("host: drain_events local_events=" + std::to_string(local_events.size()));
    auto index = static_cast<int>(events->GetSize());
    for (auto& json : local_events) {
      AppendDebugLog("host: drain_events merge_local_event bytes=" +
                     std::to_string(json.size()));
      events->SetValue(index++, ParseJsonValue(json, "local event is not valid json"));
    }
    response->SetList("events", events);
    AppendDebugLog("host: drain_events encode_response");
    auto encoded = EncodeEnvelope(MessageKind::DrainEvents, response);
    AppendDebugLog("host: drain_events encoded");
    return encoded;
  }

  std::vector<std::uint8_t> Navigate(const std::vector<std::uint8_t>& request) override {
    auto payload = RequireDictionary(
        DecodeEnvelope(MessageKind::Navigate, request), "navigate request must be a dictionary");
    const auto target_url = payload->GetString("url").ToString();
    NavigateTo(target_url);
    EnsureRuntimeInstalled();

    auto response = CefDictionaryValue::Create();
    response->SetString("url", CurrentUrl());
    response->SetValue(
        "snapshot",
        ParseJsonValue(InvokeRenderer(aegis::kOpSnapshotDom, "{}"),
                       "navigation snapshot is not valid json"));

    auto events = CefListValue::Create();
    events->SetValue(0, NavigationEvent(CurrentUrl()));
    response->SetList("events", events);
    AppendDebugLog("host: navigate encode_response");
    auto encoded = EncodeEnvelope(MessageKind::Navigate, response);
    AppendDebugLog("host: navigate encoded");
    return encoded;
  }

  std::vector<std::uint8_t> Pump(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    RequireOwnerThread();
    AegisPumpBrowserHostWindow();
    CefDoMessageLoopWork();
    return {};
  }

  void OnPrimaryBrowserCreated(CefRefPtr<CefBrowser> browser) override {
    AppendDebugLog("host: on_browser_created");
    {
      std::lock_guard lock(mutex_);
      browser_ = browser;
      request_context_ = browser->GetHost()->GetRequestContext();
      page_ready_ = false;
      renderer_ready_ = false;
      runtime_installed_ = false;
      if (auto frame = browser->GetMainFrame(); frame.get()) {
        current_url_ = frame->GetURL().ToString();
      }
      cv_.notify_all();
    }
    if (!options_.headless && browser) {
      AegisSetBrowserHostAddress(browser->GetMainFrame()->GetURL().ToString());
      AegisSetBrowserHostNavigationState(browser->CanGoBack(), browser->CanGoForward(),
                                         browser->IsLoading());
      AegisAttachBrowserToHostWindow(browser);
      AegisShowBrowserHostWindow();
    }
  }

  void OnBeforeBrowse(CefRefPtr<CefBrowser>,
                      CefRefPtr<CefFrame>,
                      CefRefPtr<CefRequest>) override {
    AppendDebugLog("host: on_before_browse");
    std::lock_guard lock(mutex_);
    page_ready_ = false;
    renderer_ready_ = false;
    runtime_installed_ = false;
  }

  void OnLoadingStateChange(CefRefPtr<CefBrowser> browser, bool is_loading) override {
    AppendDebugLog(std::string("host: on_loading_state_change loading=") +
                   (is_loading ? "true" : "false"));
    {
      std::lock_guard lock(mutex_);
      if (!browser_.get() || !browser->IsSame(browser_)) {
        return;
      }
      page_ready_ = !is_loading;
      if (!is_loading) {
        current_url_ = browser->GetMainFrame()->GetURL().ToString();
      }
      cv_.notify_all();
    }
    if (!options_.headless && browser) {
      AegisSetBrowserHostNavigationState(browser->CanGoBack(), browser->CanGoForward(),
                                         is_loading);
    }
  }

  void OnLoadEnd(CefRefPtr<CefBrowser> browser,
                 CefRefPtr<CefFrame> frame,
                 int http_status_code) override {
    AppendDebugLog(std::string("host: on_load_end status=") +
                   std::to_string(http_status_code));
    if (!frame.get() || !frame->IsMain()) {
      return;
    }
    std::lock_guard lock(mutex_);
    if (!browser_.get() || !browser->IsSame(browser_)) {
      return;
    }
    current_url_ = frame->GetURL().ToString();
  }

  void OnAddressChange(CefRefPtr<CefBrowser>,
                       CefRefPtr<CefFrame> frame,
                       const CefString& url) override {
    if (options_.headless || !frame.get() || !frame->IsMain()) {
      return;
    }
    AegisSetBrowserHostAddress(url.ToString());
  }

  void OnTitleChange(CefRefPtr<CefBrowser>,
                     const CefString& title) override {
    if (options_.headless) {
      return;
    }
    AegisSetBrowserHostTitle(title.ToString());
  }

  void OnBeforeClose(CefRefPtr<CefBrowser> browser) override {
    AppendDebugLog("host: on_before_close");
    std::lock_guard lock(mutex_);
    if (browser_.get() && browser->IsSame(browser_)) {
      browser_ = nullptr;
      request_context_ = nullptr;
      client_ = nullptr;
      page_ready_ = false;
      renderer_ready_ = false;
      runtime_installed_ = false;
      browser_closed_ = true;
      cv_.notify_all();
    }
    if (!options_.headless) {
      AegisCloseBrowserHostWindow();
    }
  }

  cef_return_value_t OnBeforeResourceLoad(CefRefPtr<CefBrowser>,
                                          CefRefPtr<CefFrame>,
                                          CefRefPtr<CefRequest> request) override {
    std::lock_guard lock(mutex_);
    if (network_overrides_.empty()) {
      return RV_CONTINUE;
    }

    CefRequest::HeaderMap headers;
    request->GetHeaderMap(headers);
    for (const auto& [header, value] : network_overrides_) {
      auto range = headers.equal_range(header);
      headers.erase(range.first, range.second);
      headers.emplace(header, value);
    }
    request->SetHeaderMap(headers);
    return RV_CONTINUE;
  }

  void OnResourceLoadComplete(CefRefPtr<CefBrowser>,
                              CefRefPtr<CefFrame>,
                              CefRefPtr<CefRequest> request,
                              CefRefPtr<CefResponse>,
                              cef_urlrequest_status_t) override {
    PushLocalEvent(NetworkEvent(request->GetIdentifier(), request->GetURL().ToString()));
  }

  bool HandleBrowserProcessMessage(CefRefPtr<CefBrowser>,
                                   CefRefPtr<CefFrame>,
                                   CefProcessId source_process,
                                   CefRefPtr<CefProcessMessage> message) {
    if (source_process != PID_RENDERER || !message.get() ||
        (message->GetName() != aegis::kAegisResponseMessage &&
         message->GetName() != aegis::kAegisLifecycleMessage)) {
      return false;
    }

    if (message->GetName() == aegis::kAegisLifecycleMessage) {
      auto args = message->GetArgumentList();
      if (args->GetString(0).ToString() == aegis::kLifecycleContextReady) {
        AppendDebugLog("host: lifecycle context_ready");
        std::lock_guard lock(mutex_);
        renderer_ready_ = true;
        const auto url = args->GetString(1).ToString();
        if (!url.empty()) {
          current_url_ = url;
        }
        cv_.notify_all();
        return true;
      }
      return false;
    }

    auto args = message->GetArgumentList();
    AppendDebugLog("host: renderer response received");
    CompleteRendererRequest(args->GetInt(0), args->GetBool(1),
                            args->GetString(2).ToString());
    return true;
  }

 private:
  void RequireOwnerThread() const {
    if (std::this_thread::get_id() != owner_thread_id_) {
      throw std::runtime_error("aegis CEF host methods must run on the owner thread");
    }
  }

  template <typename Predicate>
  void PumpUntil(Predicate predicate,
                 std::chrono::steady_clock::time_point deadline,
                 const char* timeout_message) {
    for (;;) {
      RequireOwnerThread();

      {
        std::lock_guard lock(mutex_);
        if (!startup_error_.empty()) {
          throw std::runtime_error(startup_error_);
        }
        if (predicate()) {
          return;
        }
      }

      if (std::chrono::steady_clock::now() >= deadline) {
        throw std::runtime_error(timeout_message);
      }

      AegisPumpBrowserHostWindow();
      CefDoMessageLoopWork();
      std::this_thread::sleep_for(kPumpInterval);
    }
  }

  std::vector<std::uint8_t> EncodeJsonEnvelope(MessageKind kind, const std::string& json) {
    return EncodeEnvelope(kind, ParseJsonValue(json, "renderer response is not valid json"));
  }

  CefRefPtr<CefValue> NavigationEvent(const std::string& url) {
    auto event = CefDictionaryValue::Create();
    auto body = CefDictionaryValue::Create();
    body->SetString("type", "navigation");
    body->SetString("url", url);
    event->SetDictionary("event", body);

    auto wrapped = CefValue::Create();
    wrapped->SetDictionary(event);
    return wrapped;
  }

  CefRefPtr<CefValue> NetworkEvent(std::uint64_t request_id, const std::string& url) {
    auto event = CefDictionaryValue::Create();
    auto body = CefDictionaryValue::Create();
    body->SetString("type", "network");
    body->SetString("request_id", std::to_string(request_id));
    body->SetString("url", url);
    event->SetDictionary("event", body);

    auto wrapped = CefValue::Create();
    wrapped->SetDictionary(event);
    return wrapped;
  }

  void PushLocalEvent(CefRefPtr<CefValue> event) {
    std::lock_guard lock(mutex_);
    local_events_.push_back(WriteJson(event));
  }

  std::vector<std::string> DrainLocalEvents() {
    std::lock_guard lock(mutex_);
    auto events = std::move(local_events_);
    local_events_.clear();
    return events;
  }

  void Start() {
    try {
      AppendDebugLog("host: start");
      RequireOwnerThread();

      const auto framework_binary =
          paths_.framework_dir / "Chromium Embedded Framework";
      if (!cef_load_library(framework_binary.string().c_str())) {
        throw std::runtime_error("failed to load Chromium Embedded Framework");
      }

      CefMainArgs main_args;
      CefSettings settings;
#if !defined(CEF_USE_SANDBOX)
      settings.no_sandbox = true;
#endif
      settings.external_message_pump = true;
      settings.windowless_rendering_enabled = true;
      settings.command_line_args_disabled = false;
      settings.log_severity = LOGSEVERITY_DISABLE;

      if (!options_.headless) {
        AegisInitializeBrowserHostApplication();
      }

      CefString(&settings.cache_path) = runtime_paths_.profile_dir.string();
      CefString(&settings.root_cache_path) = runtime_paths_.root_cache_dir.string();
      AppendDebugLog("host: runtime root_cache_path=" + runtime_paths_.root_cache_dir.string());
      AppendDebugLog("host: runtime cache_path=" + runtime_paths_.profile_dir.string());

      CefString(&settings.browser_subprocess_path) = paths_.helper_executable.string();
      CefString(&settings.framework_dir_path) = paths_.framework_dir.string();
      CefString(&settings.main_bundle_path) = paths_.app_bundle.string();
      CefString(&settings.resources_dir_path) = paths_.resources_dir.string();
      CefString(&settings.locales_dir_path) = paths_.locales_dir.string();

      app_ = new AegisApp(false);
      if (!CefInitialize(main_args, settings, app_.get(), nullptr)) {
        throw std::runtime_error("CefInitialize failed");
      }
      cef_initialized_ = true;
      AppendDebugLog("host: cef initialized");

      CreateBrowserOnUiThread();
      AppendDebugLog("host: create browser dispatched");

      {
        std::lock_guard lock(mutex_);
        startup_complete_ = true;
        cv_.notify_all();
      }
    } catch (const std::exception& error) {
      if (cef_initialized_) {
        CefShutdown();
        cef_unload_library();
        cef_initialized_ = false;
      }
      RemoveTreeIfExists(runtime_paths_.root_cache_dir);
      std::lock_guard lock(mutex_);
      startup_error_ = error.what();
      cv_.notify_all();
    }
  }

  void Shutdown() {
    if (!manage_cef_lifecycle_) {
      return;
    }
    if (!cef_initialized_) {
      return;
    }
    if (std::this_thread::get_id() != owner_thread_id_) {
      return;
    }

    try {
      AppendDebugLog("host: shutdown");
      const auto has_browser = [this]() {
        std::lock_guard lock(mutex_);
        return browser_.get() != nullptr;
      }();
      if (has_browser) {
        CloseBrowserOnUiThread();
        const auto deadline = std::chrono::steady_clock::now() + kShutdownTimeout;
        PumpUntil([this]() { return browser_closed_ || browser_.get() == nullptr; }, deadline,
                  "timed out waiting for browser shutdown");
      }
      CefShutdown();
      cef_unload_library();
      cef_initialized_ = false;
      RemoveTreeIfExists(runtime_paths_.root_cache_dir);
    } catch (...) {
      cef_initialized_ = false;
    }
  }

  void AttachToInitializedCef() {
    RequireOwnerThread();
    AppendDebugLog("host: attach_to_initialized_cef");
    cef_initialized_ = true;
    CreateBrowserOnUiThread();
    {
      std::lock_guard lock(mutex_);
      startup_complete_ = true;
      cv_.notify_all();
    }
  }

  void CreateBrowserOnUiThread() {
    auto task = [this]() {
      CEF_REQUIRE_UI_THREAD();
      AppendDebugLog("host: create_browser_on_ui_thread");

      CefRequestContextSettings request_context_settings;
      CefString(&request_context_settings.cache_path) = runtime_paths_.profile_dir.string();
      request_context_settings.persist_session_cookies = 1;
      request_context_ = CefRequestContext::CreateContext(request_context_settings, nullptr);
      if (!request_context_.get()) {
        throw std::runtime_error("failed to create request context");
      }

      CefBrowserSettings settings;
      settings.windowless_frame_rate = 30;

      client_ = new AegisHostClient(options_.headless,
                                    static_cast<::AegisClientDelegate*>(this), this);
      const auto initial_url = options_.start_url.empty() ? std::string(kBootstrapUrl)
                                                          : options_.start_url;

      if (options_.headless) {
        CefWindowInfo window_info;
        window_info.SetAsWindowless(kNullWindowHandle);
        window_info.runtime_style = CEF_RUNTIME_STYLE_ALLOY;
        if (!CefBrowserHost::CreateBrowser(window_info, client_, initial_url, settings, nullptr,
                                           request_context_)) {
          throw std::runtime_error("failed to create headless browser");
        }
        AppendDebugLog("host: create headless browser requested");
        return;
      }
      CefWindowInfo window_info;
      window_info.SetAsChild(AegisCreateBrowserHostView("Aegis", 1280, 800),
                             CefRect(0, 0, 1280, 800));
      window_info.runtime_style = CEF_RUNTIME_STYLE_ALLOY;
      if (!CefBrowserHost::CreateBrowser(window_info, client_, initial_url, settings, nullptr,
                                         request_context_)) {
        throw std::runtime_error("failed to create headful browser");
      }
      AppendDebugLog("host: create headful browser requested");
    };

    if (CefCurrentlyOn(TID_UI)) {
      task();
      return;
    }
    RunOnUiThreadSync(task);
  }

  void CloseBrowserOnUiThread() {
    auto task = [this]() {
      CEF_REQUIRE_UI_THREAD();
      if (browser_.get()) {
        browser_->GetHost()->CloseBrowser(true);
      }
    };
    if (CefCurrentlyOn(TID_UI)) {
      task();
      return;
    }
    RunOnUiThreadSync(task);
  }

  void RunOnUiThreadSync(const std::function<void()>& work) {
    RequireOwnerThread();
    if (CefCurrentlyOn(TID_UI)) {
      work();
      return;
    }
    auto deadline = std::chrono::steady_clock::now() + kStartupTimeout;
    auto event = CefWaitableEvent::CreateWaitableEvent(true, false);
    auto error = std::make_shared<std::exception_ptr>();
    CefPostTask(TID_UI, new UiClosureTask(work, error, event));
    PumpUntil([&event]() { return event->IsSignaled(); }, deadline,
              "timed out waiting for UI task");
    if (*error) {
      std::rethrow_exception(*error);
    }
  }

  void EnsurePageReady() {
    RequireOwnerThread();
    AppendDebugLog("host: ensure_page_ready enter");
    const auto deadline = std::chrono::steady_clock::now() + kStartupTimeout;
    PumpUntil([this]() { return startup_complete_ || !startup_error_.empty(); }, deadline,
              "timed out waiting for CEF startup");
    PumpUntil(
        [this]() {
          return browser_.get() != nullptr && renderer_ready_ && page_ready_;
        },
        deadline, "timed out waiting for browser page readiness");
    AppendDebugLog("host: ensure_page_ready complete");
  }

  void WaitForReadyLocked(std::unique_lock<std::mutex>& lock) {
    if (!startup_complete_ && startup_error_.empty()) {
      cv_.wait_for(lock, kStartupTimeout, [this] {
        return startup_complete_ || !startup_error_.empty();
      });
    }
    if (!startup_error_.empty()) {
      throw std::runtime_error(startup_error_);
    }
  }

  std::string CurrentUrl() {
    RequireOwnerThread();
    AegisPumpBrowserHostWindow();
    CefDoMessageLoopWork();
    std::lock_guard lock(mutex_);
    return current_url_;
  }

  void NavigateTo(const std::string& url) {
    {
      std::lock_guard lock(mutex_);
      if (browser_.get() != nullptr && page_ready_ && current_url_ == url) {
        AppendDebugLog("host: navigate_to skipped_same_url");
        return;
      }
    }
    {
      std::lock_guard lock(mutex_);
      page_ready_ = false;
      renderer_ready_ = false;
    }
    RunOnUiThreadSync([this, url]() {
      CEF_REQUIRE_UI_THREAD();
      if (!browser_.get()) {
        throw std::runtime_error("browser is not available");
      }
      browser_->GetMainFrame()->LoadURL(url);
    });
  }

  void EnsureRuntimeInstalled() {
    EnsurePageReady();

    bool needs_install = false;
    {
      std::lock_guard lock(mutex_);
      needs_install = !runtime_script_.empty() && !runtime_installed_;
    }
    if (!needs_install) {
      return;
    }

    InvokeRenderer(aegis::kOpInstallRuntime, runtime_script_);

    std::lock_guard lock(mutex_);
    runtime_installed_ = true;
  }

  std::string InvokeRenderer(const std::string& operation, const std::string& body) {
    EnsurePageReady();
    AppendDebugLog("host: invoke_renderer " + operation);

    const int request_id = [this] {
      std::lock_guard lock(mutex_);
      return next_request_id_++;
    }();

    RunOnUiThreadSync([this, request_id, operation, body]() {
      CEF_REQUIRE_UI_THREAD();
      if (!browser_.get()) {
        throw std::runtime_error("browser is not available");
      }
      auto frame = browser_->GetMainFrame();
      if (!frame.get()) {
        throw std::runtime_error("main frame is not available");
      }

      auto message = CefProcessMessage::Create(aegis::kAegisRequestMessage);
      auto args = message->GetArgumentList();
      args->SetInt(0, request_id);
      args->SetString(1, operation);
      args->SetString(2, body);
      frame->SendProcessMessage(PID_RENDERER, message);
    });

    const auto deadline = std::chrono::steady_clock::now() + kRendererTimeout;
    PumpUntil([this, request_id]() {
      return renderer_replies_.contains(request_id) || !startup_error_.empty();
    }, deadline, "timed out waiting for renderer response");

    RendererReply reply;
    {
      std::lock_guard lock(mutex_);
      reply = std::move(renderer_replies_.at(request_id));
      renderer_replies_.erase(request_id);
    }
    if (!reply.ok) {
      throw std::runtime_error(reply.body);
    }
    AppendDebugLog("host: invoke_renderer complete " + operation);
    return reply.body;
  }

  void CompleteRendererRequest(int request_id, bool ok, std::string body) {
    std::lock_guard lock(mutex_);
    renderer_replies_[request_id] = RendererReply{
        .ok = ok,
        .body = std::move(body),
    };
    cv_.notify_all();
  }

  void ReplaceNetworkOverrides(CefRefPtr<CefDictionaryValue> session) {
    std::vector<std::pair<std::string, std::string>> overrides;
    if (session->HasKey("network_overrides")) {
      auto list = session->GetList("network_overrides");
      for (std::size_t index = 0; index < list->GetSize(); ++index) {
        auto override_value =
            RequireDictionary(list->GetValue(static_cast<int>(index)),
                              "network override must be a dictionary");
        overrides.emplace_back(override_value->GetString("header").ToString(),
                               override_value->GetString("value").ToString());
      }
    }

    std::lock_guard lock(mutex_);
    network_overrides_ = std::move(overrides);
  }

  void ReplaceCookies(CefRefPtr<CefDictionaryValue> session) {
    auto manager = request_context_->GetCookieManager(nullptr);

    auto clear_event = CefWaitableEvent::CreateWaitableEvent(true, false);
    manager->DeleteCookies("", "", new DeleteCookieSignal(clear_event));
    clear_event->Wait();

    if (!session->HasKey("cookies")) {
      return;
    }

    auto cookies = session->GetList("cookies");
    for (std::size_t index = 0; index < cookies->GetSize(); ++index) {
      auto cookie_value = RequireDictionary(
          cookies->GetValue(static_cast<int>(index)), "cookie must be a dictionary");

      CefCookie cookie{};
      CefString(&cookie.name) = cookie_value->GetString("name");
      CefString(&cookie.value) = cookie_value->GetString("value");
      CefString(&cookie.domain) = cookie_value->GetString("domain");
      CefString(&cookie.path) =
          cookie_value->HasKey("path") ? cookie_value->GetString("path") : CefString("/");
      cookie.secure = cookie_value->HasKey("secure") && cookie_value->GetBool("secure");
      cookie.httponly =
          cookie_value->HasKey("http_only") && cookie_value->GetBool("http_only");

      if (cookie_value->HasKey("expires_unix")) {
        cookie.has_expires = true;
        cef_time_t expires{};
        cef_time_from_timet(
            static_cast<time_t>(cookie_value->GetDouble("expires_unix")), &expires);
        cef_time_to_basetime(&expires, &cookie.expires);
      }

      auto set_event = CefWaitableEvent::CreateWaitableEvent(true, false);
      manager->SetCookie(CookieUrl(cookie_value), cookie, new SetCookieSignal(set_event));
      set_event->Wait();
    }

    auto flush_event = CefWaitableEvent::CreateWaitableEvent(true, false);
    manager->FlushStore(new CompletionSignal(flush_event));
    flush_event->Wait();
  }

  CefRefPtr<CefListValue> SnapshotCookies() {
    std::vector<CefCookie> cookies;
    auto event = CefWaitableEvent::CreateWaitableEvent(true, false);
    request_context_->GetCookieManager(nullptr)->VisitAllCookies(
        new CookieCollector(&cookies, event));
    event->Wait();

    auto list = CefListValue::Create();
    for (std::size_t index = 0; index < cookies.size(); ++index) {
      const auto& cookie = cookies[index];
      auto entry = CefDictionaryValue::Create();
      entry->SetString("name", CefString(&cookie.name));
      entry->SetString("value", CefString(&cookie.value));
      entry->SetString("domain", CefString(&cookie.domain));
      entry->SetString("path", CefString(&cookie.path));
      entry->SetBool("secure", cookie.secure != 0);
      entry->SetBool("http_only", cookie.httponly != 0);
      if (cookie.has_expires) {
        cef_time_t expires{};
        cef_time_from_basetime(cookie.expires, &expires);
        time_t expires_unix = 0;
        cef_time_to_timet(&expires, &expires_unix);
        entry->SetDouble("expires_unix", static_cast<double>(expires_unix));
      }
      list->SetDictionary(static_cast<int>(index), entry);
    }
    return list;
  }

  CefRefPtr<CefListValue> SnapshotNetworkOverrides() {
    std::lock_guard lock(mutex_);
    auto list = CefListValue::Create();
    for (std::size_t index = 0; index < network_overrides_.size(); ++index) {
      auto entry = CefDictionaryValue::Create();
      entry->SetString("header", network_overrides_[index].first);
      entry->SetString("value", network_overrides_[index].second);
      list->SetDictionary(static_cast<int>(index), entry);
    }
    return list;
  }

  const BrowserOptions options_;
  const HostPaths paths_;
  const HostRuntimePaths runtime_paths_;
  const std::thread::id owner_thread_id_;
  const bool manage_cef_lifecycle_;

  CefRefPtr<AegisApp> app_;
  CefRefPtr<AegisClient> client_;

  mutable std::mutex mutex_;
  std::condition_variable cv_;
  bool cef_initialized_ = false;
  bool startup_complete_ = false;
  bool page_ready_ = false;
  bool renderer_ready_ = false;
  bool runtime_installed_ = false;
  bool browser_closed_ = false;
  std::string startup_error_;
  std::string current_url_ = "about:blank";
  int next_request_id_ = 1;
  std::string runtime_script_;
  std::vector<std::pair<std::string, std::string>> network_overrides_;
  std::vector<std::string> local_events_;
  std::map<int, RendererReply> renderer_replies_;

  CefRefPtr<CefBrowser> browser_;
  CefRefPtr<CefRequestContext> request_context_;
};

bool AegisHostClient::OnProcessMessageReceived(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefProcessId source_process,
    CefRefPtr<CefProcessMessage> message) {
  return host_ &&
         host_->HandleBrowserProcessMessage(browser, frame, source_process, message);
}

template <typename Method>
AegisHostStatus Dispatch(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output,
    Method method) {
  try {
    if (ctx == nullptr) {
      throw std::runtime_error("cef host context is null");
    }

    auto* host = static_cast<CefHost*>(ctx);
    WriteOutput((host->*method)(CopyInput(input_ptr, input_len)), output);
    return AEGIS_HOST_OK;
  } catch (const std::exception& error) {
    const auto message = std::string(error.what());
    WriteOutput(std::vector<std::uint8_t>(message.begin(), message.end()), output);
    return AEGIS_HOST_ERROR;
  }
}

AegisHostStatus InstallRuntime(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::InstallRuntime);
}

AegisHostStatus EvalJs(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::EvalJs);
}

AegisHostStatus SendBatch(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SendBatch);
}

AegisHostStatus SnapshotDom(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SnapshotDom);
}

AegisHostStatus InjectSession(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::InjectSession);
}

AegisHostStatus SnapshotSession(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SnapshotSession);
}

AegisHostStatus DrainEvents(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::DrainEvents);
}

AegisHostStatus Navigate(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::Navigate);
}

AegisHostStatus Pump(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::Pump);
}

void FreeBuffer(AegisHostHandle, AegisHostBuffer buffer) {
  delete[] buffer.ptr;
}

}  // namespace

AegisHostFunctionTable ExportFunctionTable() {
  return AegisHostFunctionTable{
      .install_runtime = InstallRuntime,
      .eval_js = EvalJs,
      .send_batch = SendBatch,
      .snapshot_dom = SnapshotDom,
      .inject_session = InjectSession,
      .snapshot_session = SnapshotSession,
      .drain_events = DrainEvents,
      .navigate = Navigate,
      .pump = Pump,
      .free_buffer = FreeBuffer,
  };
}

}  // namespace aegis

namespace aegis {

bool RunEmbeddedHostOperation(const std::vector<std::uint8_t>& config,
                              EmbeddedHostOperation operation,
                              const std::vector<std::uint8_t>& request,
                              std::vector<std::uint8_t>* response,
                              std::string* error) {
  try {
    if (response == nullptr) {
      throw std::runtime_error("response buffer is null");
    }
    response->clear();

    AegisCefHost host(ParseBrowserOptions(config), false);
    host.WaitForReady();

    if (operation != EmbeddedHostOperation::InstallRuntime) {
      const auto runtime_path = std::filesystem::current_path() / "assets" / "js" / "aegis_runtime.js";
      if (std::filesystem::exists(runtime_path)) {
        std::ifstream runtime_input(runtime_path, std::ios::binary);
        if (runtime_input.is_open()) {
          const std::string script((std::istreambuf_iterator<char>(runtime_input)),
                                   std::istreambuf_iterator<char>());
          auto runtime_value = CefValue::Create();
          runtime_value->SetString(script);
          host.InstallRuntime(EncodeEnvelope(MessageKind::InstallRuntime, runtime_value));
        }
      }
    }

    switch (operation) {
      case EmbeddedHostOperation::InstallRuntime:
        *response = host.InstallRuntime(request);
        break;
      case EmbeddedHostOperation::EvalJs:
        *response = host.EvalJs(request);
        break;
      case EmbeddedHostOperation::SendBatch:
        *response = host.SendBatch(request);
        break;
      case EmbeddedHostOperation::SnapshotDom:
        *response = host.SnapshotDom(request);
        break;
      case EmbeddedHostOperation::InjectSession:
        *response = host.InjectSession(request);
        break;
      case EmbeddedHostOperation::SnapshotSession:
        *response = host.SnapshotSession(request);
        break;
      case EmbeddedHostOperation::DrainEvents:
        *response = host.DrainEvents(request);
        break;
      case EmbeddedHostOperation::Navigate:
        *response = host.Navigate(request);
        break;
      default:
        throw std::runtime_error("unsupported embedded host operation");
    }

    return true;
  } catch (const std::exception& ex) {
    if (error != nullptr) {
      *error = ex.what();
    }
    return false;
  }
}

}  // namespace aegis

extern "C" AegisHostHandle aegis_create_host(const std::uint8_t* input_ptr, std::size_t input_len) {
  try {
    auto host = std::make_unique<aegis::AegisCefHost>(
        aegis::ParseBrowserOptions(aegis::CopyInput(input_ptr, input_len)));
    host->WaitForReady();
    return host.release();
  } catch (...) {
    return nullptr;
  }
}

extern "C" void aegis_destroy_host(AegisHostHandle handle) {
  delete static_cast<aegis::CefHost*>(handle);
}

extern "C" AegisHostFunctionTable aegis_get_function_table(void) {
  return aegis::ExportFunctionTable();
}
