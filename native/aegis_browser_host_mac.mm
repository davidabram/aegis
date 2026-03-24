#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#import <objc/runtime.h>

#include "aegis_native_mac.h"
#include "include/cef_application_mac.h"

#include <stdexcept>
#include <string>

@interface NSAlert (AegisSuppression)
- (NSModalResponse)aegis_runModal;
@end

@implementation NSAlert (AegisSuppression)
- (NSModalResponse)aegis_runModal {
  return NSModalResponseCancel;
}
@end

void AegisInstallModalAlertSuppression() {
  static dispatch_once_t once_token;
  dispatch_once(&once_token, ^{
    Method original = class_getInstanceMethod([NSAlert class], @selector(runModal));
    Method replacement = class_getInstanceMethod([NSAlert class], @selector(aegis_runModal));
    method_exchangeImplementations(original, replacement);
  });
}

#include "aegis_browser_host_mac.inc"
