#ifndef AEGIS_NATIVE_AEGIS_NATIVE_MAC_H_
#define AEGIS_NATIVE_AEGIS_NATIVE_MAC_H_

#include <string>

#include "include/cef_browser.h"
#include "include/internal/cef_types.h"

std::string AegisStandaloneRootCachePath();
std::string AegisStandaloneCachePath();
void AegisInitializeBrowserHostApplication();
CefWindowHandle AegisCreateBrowserHostView(const std::string& title,
                                           int width,
                                           int height);
void AegisShowBrowserHostWindow();
void AegisSetBrowserHostTitle(const std::string& title);
void AegisSetBrowserHostAddress(const std::string& url);
void AegisSetBrowserHostNavigationState(bool can_go_back,
                                        bool can_go_forward,
                                        bool is_loading);
void AegisAttachBrowserToHostWindow(CefRefPtr<CefBrowser> browser);
void AegisCloseBrowserHostWindow();
void AegisPumpBrowserHostWindow();

#endif  // AEGIS_NATIVE_AEGIS_NATIVE_MAC_H_
