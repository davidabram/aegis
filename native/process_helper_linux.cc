#include "aegis_app.h"
#include "include/cef_app.h"
#if __has_include("include/wrapper/cef_library_loader.h")
#include "include/wrapper/cef_library_loader.h"
#define AEGIS_HAS_CEF_LIBRARY_LOADER 1
#endif

int main(int argc, char* argv[]) {
#if defined(AEGIS_HAS_CEF_LIBRARY_LOADER)
  CefScopedLibraryLoader loader;
  if (!loader.LoadInHelper()) {
    return 1;
  }
#endif

  CefMainArgs main_args(argc, argv);
  CefRefPtr<AegisApp> app(new AegisApp(false));
  return CefExecuteProcess(main_args, app.get(), nullptr);
}
