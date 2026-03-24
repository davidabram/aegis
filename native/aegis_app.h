#ifndef AEGIS_NATIVE_AEGIS_APP_H_
#define AEGIS_NATIVE_AEGIS_APP_H_

#include <string>

#include "aegis_client.h"
#include "aegis_state_paths.h"
#include "include/cef_app.h"
#include "include/cef_request_context.h"
#include "include/cef_render_process_handler.h"

class AegisApp : public CefApp,
                 public CefBrowserProcessHandler,
                 public CefRenderProcessHandler,
                 public AegisClientDelegate {
 public:
  explicit AegisApp(bool launch_browser_on_context_initialized = false,
                    std::string startup_url = {});

  CefRefPtr<CefBrowserProcessHandler> GetBrowserProcessHandler() override {
    return this;
  }
  CefRefPtr<CefRenderProcessHandler> GetRenderProcessHandler() override {
    return this;
  }

  void OnBeforeCommandLineProcessing(const CefString& process_type,
                                     CefRefPtr<CefCommandLine> command_line) override;
  void OnBeforeChildProcessLaunch(
      CefRefPtr<CefCommandLine> command_line) override;
  bool OnAlreadyRunningAppRelaunch(
      CefRefPtr<CefCommandLine> command_line,
      const CefString& current_directory) override;
  void OnScheduleMessagePumpWork(int64_t delay_ms) override;
  void OnContextInitialized() override;
  void OnContextCreated(CefRefPtr<CefBrowser> browser,
                        CefRefPtr<CefFrame> frame,
                        CefRefPtr<CefV8Context> context) override;
  bool OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                CefProcessId source_process,
                                CefRefPtr<CefProcessMessage> message) override;
  void OnPrimaryBrowserCreated(CefRefPtr<CefBrowser> browser) override;
  void OnLoadingStateChange(CefRefPtr<CefBrowser> browser,
                            bool is_loading) override;
  void OnAddressChange(CefRefPtr<CefBrowser> browser,
                       CefRefPtr<CefFrame> frame,
                       const CefString& url) override;
  void OnTitleChange(CefRefPtr<CefBrowser> browser,
                     const CefString& title) override;
  void OnBeforeClose(CefRefPtr<CefBrowser> browser) override;

  const AegisRuntimeSessionPaths& runtime_session_paths() const {
    return runtime_session_paths_;
  }

 private:
  void CreateHeadfulBrowser(const std::string& url);

  const bool launch_browser_on_context_initialized_;
  CefRefPtr<CefBrowser> primary_browser_;
  std::string startup_url_;
  std::string pending_startup_url_;
  AegisRuntimeSessionPaths runtime_session_paths_;
  CefRefPtr<CefRequestContext> request_context_;

  IMPLEMENT_REFCOUNTING(AegisApp);
};

#endif
