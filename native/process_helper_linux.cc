#include "aegis_app.h"
#include "include/cef_app.h"

int main(int argc, char* argv[]) {
  CefMainArgs main_args(argc, argv);
  CefRefPtr<AegisApp> app(new AegisApp(false));
  return CefExecuteProcess(main_args, app.get(), nullptr);
}
