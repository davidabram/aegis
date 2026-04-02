#include "aegis_app.h"
#include "include/cef_app.h"
#include "include/wrapper/cef_library_loader.h"

int main(int argc, char* argv[]) {
  CefScopedLibraryLoader loader;
  if (!loader.LoadInHelper()) {
    return 1;
  }

  CefMainArgs main_args(argc, argv);
  CefRefPtr<AegisApp> app(new AegisApp(false));
  return CefExecuteProcess(main_args, app.get(), nullptr);
}
