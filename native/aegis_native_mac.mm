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

// ═══════════════════════════════════════════════════════════════════════════════
// DESIGN SYSTEM — Vercel/shadcn-inspired browser chrome
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Layout tokens ───────────────────────────────────────────────────────────

static const CGFloat kToolbarHeight       = 82.0;
static const CGFloat kTabHeight           = 28.0;
static const CGFloat kTabRadius           = 8.0;
static const CGFloat kTabLeftInset        = 76.0;  // clears traffic lights
static const CGFloat kTabHPad             = 12.0;
static const CGFloat kTabInitialWidth     = 200.0;
static const CGFloat kNavBtnSize          = 28.0;
static const CGFloat kNavBtnRadius        = 6.0;
static const CGFloat kNavBtnGap           = 4.0;
static const CGFloat kNavLeftInset        = 14.0;
static const CGFloat kAddrHeight          = 32.0;
static const CGFloat kAddrRadius          = 8.0;
static const CGFloat kAddrHPad            = 10.0;
static const CGFloat kAddrGapFromNav      = 10.0;
static const CGFloat kAddrRightInset      = 14.0;
static const CGFloat kProgressHeight      = 2.0;
static const CGFloat kSeparatorHeight     = 1.0;

// ─── Color helpers ───────────────────────────────────────────────────────────

static NSColor* AegisColor(CGFloat r, CGFloat g, CGFloat b, CGFloat a) {
  return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
}

static NSColor* AegisWindowBg(void)          { return AegisColor(0.975, 0.975, 0.975, 1.0); }
static NSColor* AegisActiveTabBorder(void)   { return AegisColor(0.0, 0.0, 0.0, 0.08); }
static NSColor* AegisPrimaryText(void)       { return AegisColor(0.10, 0.10, 0.10, 1.0); }
static NSColor* AegisSecondaryText(void)     { return AegisColor(0.0, 0.0, 0.0, 0.50); }
static NSColor* AegisPlaceholderText(void)   { return AegisColor(0.0, 0.0, 0.0, 0.32); }
static NSColor* AegisAddrBg(void)            { return AegisColor(0.0, 0.0, 0.0, 0.055); }
static NSColor* AegisAddrHoverBg(void)       { return AegisColor(0.0, 0.0, 0.0, 0.075); }
static NSColor* AegisAddrFocusBg(void)       { return [NSColor whiteColor]; }
static NSColor* AegisAddrBorder(void)        { return AegisColor(0.0, 0.0, 0.0, 0.09); }
static NSColor* AegisAddrFocusBorder(void)   { return AegisColor(0.23, 0.51, 0.97, 0.50); }
static NSColor* AegisBtnHoverBg(void)        { return AegisColor(0.0, 0.0, 0.0, 0.06); }
static NSColor* AegisBtnPressedBg(void)      { return AegisColor(0.0, 0.0, 0.0, 0.10); }
static NSColor* AegisSeparator(void)         { return AegisColor(0.0, 0.0, 0.0, 0.12); }
static NSColor* AegisAccent(void)            { return AegisColor(0.23, 0.51, 0.97, 1.0); }
static NSColor* AegisNavIconDefault(void)    { return AegisColor(0.25, 0.25, 0.25, 1.0); }
static NSColor* AegisNavIconActive(void)     { return AegisColor(0.12, 0.12, 0.12, 1.0); }
static NSColor* AegisLockIcon(void)          { return AegisColor(0.0, 0.0, 0.0, 0.36); }

// ─── AegisNavButton ──────────────────────────────────────────────────────────

@interface AegisNavButton : NSButton
- (instancetype)initWithSymbolName:(NSString*)name;
- (void)setSymbolName:(NSString*)name;
@end

@implementation AegisNavButton {
  NSTrackingArea* _trackingArea;
  BOOL _hovered;
}

- (instancetype)initWithSymbolName:(NSString*)name {
  self = [super initWithFrame:NSMakeRect(0, 0, kNavBtnSize, kNavBtnSize)];
  if (self) {
    [self setButtonType:NSButtonTypeMomentaryChange];
    [self setBezelStyle:NSBezelStyleInline];
    [self setBordered:NO];
    [self setTitle:@""];
    [self setImagePosition:NSImageOnly];
    [self setImageScaling:NSImageScaleProportionallyDown];
    [self setWantsLayer:YES];
    self.layer.cornerRadius = kNavBtnRadius;
    [self setSymbolName:name];
  }
  return self;
}

- (void)setSymbolName:(NSString*)name {
  NSImageSymbolConfiguration* config = [NSImageSymbolConfiguration
      configurationWithPointSize:13.0
                          weight:NSFontWeightMedium
                           scale:NSImageSymbolScaleMedium];
  NSImage* image = [NSImage imageWithSystemSymbolName:name
                                 accessibilityDescription:name];
  if (image) {
    image = [image imageWithSymbolConfiguration:config];
    [image setTemplate:YES];
    [self setImage:image];
    [self setTitle:@""];
    [self setImagePosition:NSImageOnly];
  }
  [self setContentTintColor:AegisNavIconDefault()];
}

- (void)setEnabled:(BOOL)enabled {
  [super setEnabled:enabled];
  self.alphaValue = enabled ? 1.0 : 0.30;
  if (!enabled) {
    _hovered = NO;
    self.layer.backgroundColor = nil;
  }
}

- (void)updateTrackingAreas {
  [super updateTrackingAreas];
  if (_trackingArea) {
    [self removeTrackingArea:_trackingArea];
  }
  _trackingArea = [[NSTrackingArea alloc]
      initWithRect:self.bounds
           options:(NSTrackingMouseEnteredAndExited | NSTrackingActiveInActiveApp)
             owner:self
          userInfo:nil];
  [self addTrackingArea:_trackingArea];
}

- (void)mouseEntered:(NSEvent*)event {
  (void)event;
  if (!self.isEnabled) return;
  _hovered = YES;
  self.layer.backgroundColor = AegisBtnHoverBg().CGColor;
  [self setContentTintColor:AegisNavIconActive()];
}

- (void)mouseExited:(NSEvent*)event {
  (void)event;
  _hovered = NO;
  self.layer.backgroundColor = nil;
  [self setContentTintColor:AegisNavIconDefault()];
}

- (void)mouseDown:(NSEvent*)event {
  if (!self.isEnabled) return;
  self.layer.backgroundColor = AegisBtnPressedBg().CGColor;
  [super mouseDown:event];
  // Restore after tracking loop completes (mouse up).
  NSPoint loc = [self convertPoint:[self.window mouseLocationOutsideOfEventStream]
                          fromView:nil];
  if (NSPointInRect(loc, self.bounds)) {
    _hovered = YES;
    self.layer.backgroundColor = AegisBtnHoverBg().CGColor;
  } else {
    _hovered = NO;
    self.layer.backgroundColor = nil;
  }
  [self setContentTintColor:AegisNavIconDefault()];
}

@end

// ─── AegisAddressContainer ───────────────────────────────────────────────────

@interface AegisAddressContainer : NSView
@property (nonatomic, assign) BOOL focused;
@end

@implementation AegisAddressContainer {
  NSTrackingArea* _trackingArea;
  BOOL _hovered;
}

- (instancetype)initWithFrame:(NSRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    [self setWantsLayer:YES];
    self.layer.cornerRadius = kAddrRadius;
    self.layer.borderWidth = 1.0;
    self.layer.masksToBounds = YES;
    [self applyDefaultStyle];
  }
  return self;
}

- (void)applyDefaultStyle {
  self.layer.backgroundColor = AegisAddrBg().CGColor;
  self.layer.borderColor = AegisAddrBorder().CGColor;
  self.layer.borderWidth = 1.0;
}

- (void)applyHoverStyle {
  self.layer.backgroundColor = AegisAddrHoverBg().CGColor;
  self.layer.borderColor = AegisAddrBorder().CGColor;
  self.layer.borderWidth = 1.0;
}

- (void)applyFocusedStyle {
  self.layer.backgroundColor = AegisAddrFocusBg().CGColor;
  self.layer.borderColor = AegisAddrFocusBorder().CGColor;
  self.layer.borderWidth = 1.5;
}

- (void)refreshStyle {
  if (_focused) {
    [self applyFocusedStyle];
  } else if (_hovered) {
    [self applyHoverStyle];
  } else {
    [self applyDefaultStyle];
  }
}

- (void)setFocused:(BOOL)focused {
  _focused = focused;
  [self refreshStyle];
}

- (void)updateTrackingAreas {
  [super updateTrackingAreas];
  if (_trackingArea) {
    [self removeTrackingArea:_trackingArea];
  }
  _trackingArea = [[NSTrackingArea alloc]
      initWithRect:self.bounds
           options:(NSTrackingMouseEnteredAndExited | NSTrackingActiveInActiveApp)
             owner:self
          userInfo:nil];
  [self addTrackingArea:_trackingArea];
}

- (void)mouseEntered:(NSEvent*)event {
  (void)event;
  _hovered = YES;
  [self refreshStyle];
}

- (void)mouseExited:(NSEvent*)event {
  (void)event;
  _hovered = NO;
  [self refreshStyle];
}

@end

// ─── AegisAddressField ──────────────────────────────────────────────────────

@interface AegisAddressField : NSTextField
@property (nonatomic, assign) AegisAddressContainer* addressContainer;
@end

@implementation AegisAddressField

- (BOOL)becomeFirstResponder {
  BOOL result = [super becomeFirstResponder];
  if (result && self.addressContainer) {
    [self.addressContainer setFocused:YES];
    // Select all text when focused, after field editor is installed.
    dispatch_async(dispatch_get_main_queue(), ^{
      NSText* editor = [[self window] fieldEditor:NO forObject:self];
      if (editor) {
        [editor selectAll:nil];
      }
    });
  }
  return result;
}

@end

// ─── AegisBrowserChromeView ─────────────────────────────────────────────────

@interface AegisBrowserChromeView : NSView
@end

@implementation AegisBrowserChromeView
- (BOOL)isOpaque {
  return YES;
}

- (void)drawRect:(NSRect)dirtyRect {
  [AegisWindowBg() setFill];
  NSRectFill(dirtyRect);
}
@end

// ─── Controller forward declaration ─────────────────────────────────────────

@interface AegisBrowserWindowController : NSObject <NSWindowDelegate, NSTextFieldDelegate>
- (instancetype)initWithStatePointer:(void*)state;
- (void)navigateBack:(id)sender;
- (void)navigateForward:(id)sender;
- (void)reloadPage:(id)sender;
- (void)submitLocation:(id)sender;
- (void)updateAddress:(NSString*)address;
- (void)updateTitle:(NSString*)title;
- (void)updateNavigationButtonsWithCanGoBack:(BOOL)canGoBack
                                 canGoForward:(BOOL)canGoForward
                                    isLoading:(BOOL)isLoading;
- (void)unfocusAddressBar;
@end

// ═══════════════════════════════════════════════════════════════════════════════
// HOST STATE & WINDOW CONSTRUCTION
// ═══════════════════════════════════════════════════════════════════════════════

namespace {

struct BrowserHostState {
  NSWindow* window = nil;
  NSView* root_view = nil;

  // Toolbar
  NSVisualEffectView* toolbar_view = nil;
  NSView* separator_view = nil;

  // Tab strip
  NSView* tab_bg = nil;
  NSTextField* tab_title_label = nil;
  AegisNavButton* new_tab_button = nil;

  // Navigation
  AegisNavButton* back_button = nil;
  AegisNavButton* forward_button = nil;
  AegisNavButton* reload_button = nil;

  // Address bar
  AegisAddressContainer* address_container = nil;
  NSImageView* lock_icon = nil;
  AegisAddressField* address_field = nil;

  // Progress
  NSView* progress_view = nil;

  // Web content
  NSView* web_container_view = nil;

  // Controller
  AegisBrowserWindowController* controller = nil;

  // Browser state
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

// ─── Window & chrome construction ────────────────────────────────────────────

void EnsureHostWindow(const std::string& title, int width, int height) {
  auto& s = HostState();
  if (s.window != nil && s.web_container_view != nil) {
    return;
  }

  const CGFloat w = static_cast<CGFloat>(width);
  const CGFloat h = static_cast<CGFloat>(height);

  // ── Window ─────────────────────────────────────────────────────────────

  s.window = [[NSWindow alloc]
      initWithContentRect:NSMakeRect(0, 0, w, h)
                styleMask:(NSWindowStyleMaskTitled |
                           NSWindowStyleMaskClosable |
                           NSWindowStyleMaskMiniaturizable |
                           NSWindowStyleMaskResizable |
                           NSWindowStyleMaskFullSizeContentView)
                  backing:NSBackingStoreBuffered
                    defer:NO];
  if (s.window == nil) {
    throw std::runtime_error("failed to create browser host window");
  }

  s.current_title = title;
  s.current_url = title;
  [s.window setTitlebarAppearsTransparent:YES];
  [s.window setTitleVisibility:NSWindowTitleHidden];
  [s.window setMovableByWindowBackground:YES];
  [s.window setReleasedWhenClosed:NO];
  [s.window setBackgroundColor:AegisWindowBg()];
  [s.window setMinSize:NSMakeSize(520, 400)];
  [s.window center];

  // ── Root view ──────────────────────────────────────────────────────────

  s.root_view = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, w, h)];
  [s.root_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  [s.window setContentView:s.root_view];

  // ── Toolbar (NSVisualEffectView — forced light) ─────────────────────

  s.toolbar_view = [[NSVisualEffectView alloc]
      initWithFrame:NSMakeRect(0, h - kToolbarHeight, w, kToolbarHeight)];
  [s.toolbar_view setMaterial:NSVisualEffectMaterialTitlebar];
  [s.toolbar_view setBlendingMode:NSVisualEffectBlendingModeWithinWindow];
  [s.toolbar_view setState:NSVisualEffectStateActive];
  [s.toolbar_view setEmphasized:YES];
  // Force light appearance on chrome only — web content uses system setting.
  [s.toolbar_view setAppearance:[NSAppearance appearanceNamed:NSAppearanceNameAqua]];
  [s.toolbar_view setAutoresizingMask:NSViewWidthSizable | NSViewMinYMargin];
  [s.root_view addSubview:s.toolbar_view];

  // ── Separator ──────────────────────────────────────────────────────────

  s.separator_view = [[NSView alloc]
      initWithFrame:NSMakeRect(0, 0, w, kSeparatorHeight)];
  [s.separator_view setWantsLayer:YES];
  s.separator_view.layer.backgroundColor = AegisSeparator().CGColor;
  [s.separator_view setAutoresizingMask:NSViewWidthSizable];
  [s.toolbar_view addSubview:s.separator_view];

  // ── Progress indicator ─────────────────────────────────────────────────

  s.progress_view = [[NSView alloc]
      initWithFrame:NSMakeRect(0, 0, 0, kProgressHeight)];
  [s.progress_view setWantsLayer:YES];
  s.progress_view.layer.backgroundColor = AegisAccent().CGColor;
  s.progress_view.layer.cornerRadius = 1.0;
  [s.progress_view setAutoresizingMask:NSViewMaxXMargin];
  s.progress_view.hidden = YES;
  [s.toolbar_view addSubview:s.progress_view positioned:NSWindowAbove
                                             relativeTo:s.separator_view];

  // ════════════════════════════════════════════════════════════════════════
  // TAB STRIP — top region of the toolbar
  // ════════════════════════════════════════════════════════════════════════

  // Vertical layout within toolbar (bottom-up coordinates):
  //   y=0          : separator (1px)
  //   y=1..2       : progress bar (2px)
  //   y=6..40      : nav/address row center region
  //   y=46..76     : tab strip region
  //   y=82         : toolbar top (window top)

  // Tab strip region: from y=44 to y=78 (34px band)
  // Tab items (28px): centered → y = 44 + (34-28)/2 = 47
  const CGFloat tab_strip_bottom = 44.0;
  const CGFloat tab_strip_band   = 34.0;
  const CGFloat tab_item_y = tab_strip_bottom + (tab_strip_band - kTabHeight) / 2.0;

  // Active tab background
  s.tab_bg = [[NSView alloc]
      initWithFrame:NSMakeRect(kTabLeftInset, tab_item_y, kTabInitialWidth, kTabHeight)];
  [s.tab_bg setWantsLayer:YES];
  s.tab_bg.layer.cornerRadius = kTabRadius;
  s.tab_bg.layer.backgroundColor = [NSColor whiteColor].CGColor;
  s.tab_bg.layer.opaque = YES;
  s.tab_bg.layer.borderWidth = 0.5;
  s.tab_bg.layer.borderColor = AegisActiveTabBorder().CGColor;
  s.tab_bg.layer.shadowColor = [NSColor blackColor].CGColor;
  s.tab_bg.layer.shadowOffset = CGSizeMake(0, -1.0);
  s.tab_bg.layer.shadowRadius = 3.0;
  s.tab_bg.layer.shadowOpacity = 0.08;
  s.tab_bg.layer.masksToBounds = NO;
  [s.toolbar_view addSubview:s.tab_bg];

  // Tab title
  s.tab_title_label = [NSTextField labelWithString:@"Aegis"];
  [s.tab_title_label setFrame:NSMakeRect(kTabHPad, 4.0,
                                          kTabInitialWidth - kTabHPad * 2.0,
                                          kTabHeight - 8.0)];
  s.tab_title_label.font = [NSFont systemFontOfSize:12.5
                                              weight:NSFontWeightMedium];
  s.tab_title_label.textColor = AegisPrimaryText();
  s.tab_title_label.lineBreakMode = NSLineBreakByTruncatingTail;
  [s.tab_title_label setAutoresizingMask:NSViewWidthSizable];
  [s.tab_bg addSubview:s.tab_title_label];

  // New-tab button (+)
  s.new_tab_button = [[AegisNavButton alloc] initWithSymbolName:@"plus"];
  const CGFloat ntb_x = kTabLeftInset + kTabInitialWidth + 6.0;
  const CGFloat ntb_size = 22.0;
  const CGFloat ntb_y = tab_item_y + (kTabHeight - ntb_size) / 2.0;
  [s.new_tab_button setFrame:NSMakeRect(ntb_x, ntb_y, ntb_size, ntb_size)];
  s.new_tab_button.layer.cornerRadius = ntb_size / 2.0;
  {
    NSImageSymbolConfiguration* cfg = [NSImageSymbolConfiguration
        configurationWithPointSize:10.0 weight:NSFontWeightMedium];
    NSImage* img = [[NSImage imageWithSystemSymbolName:@"plus"
                                 accessibilityDescription:@"New tab"]
        imageWithSymbolConfiguration:cfg];
    [s.new_tab_button setImage:img];
  }
  [s.new_tab_button setContentTintColor:AegisSecondaryText()];
  [s.new_tab_button setEnabled:NO]; // placeholder for now
  s.new_tab_button.alphaValue = 0.5;
  [s.toolbar_view addSubview:s.new_tab_button];

  // ════════════════════════════════════════════════════════════════════════
  // NAVIGATION CLUSTER — bottom-left of toolbar
  // ════════════════════════════════════════════════════════════════════════

  // Nav row vertical center: y region 2..42 → center at 22
  const CGFloat nav_center_y = 22.0;
  const CGFloat nav_btn_y = nav_center_y - kNavBtnSize / 2.0;

  // Back
  s.back_button = [[AegisNavButton alloc] initWithSymbolName:@"chevron.left"];
  CGFloat bx = kNavLeftInset;
  [s.back_button setFrame:NSMakeRect(bx, nav_btn_y, kNavBtnSize, kNavBtnSize)];
  [s.toolbar_view addSubview:s.back_button];

  // Forward
  s.forward_button = [[AegisNavButton alloc] initWithSymbolName:@"chevron.right"];
  bx += kNavBtnSize + kNavBtnGap;
  [s.forward_button setFrame:NSMakeRect(bx, nav_btn_y, kNavBtnSize, kNavBtnSize)];
  [s.toolbar_view addSubview:s.forward_button];

  // Reload / Stop
  s.reload_button = [[AegisNavButton alloc] initWithSymbolName:@"arrow.clockwise"];
  bx += kNavBtnSize + kNavBtnGap;
  [s.reload_button setFrame:NSMakeRect(bx, nav_btn_y, kNavBtnSize, kNavBtnSize)];
  [s.toolbar_view addSubview:s.reload_button];

  // ════════════════════════════════════════════════════════════════════════
  // ADDRESS BAR — central focal element
  // ════════════════════════════════════════════════════════════════════════

  const CGFloat addr_x = bx + kNavBtnSize + kAddrGapFromNav;
  const CGFloat addr_w = w - addr_x - kAddrRightInset;
  const CGFloat addr_y = nav_center_y - kAddrHeight / 2.0;

  s.address_container = [[AegisAddressContainer alloc]
      initWithFrame:NSMakeRect(addr_x, addr_y, addr_w, kAddrHeight)];
  [s.address_container setAutoresizingMask:NSViewWidthSizable];
  [s.toolbar_view addSubview:s.address_container];

  // Lock icon
  {
    NSImageSymbolConfiguration* cfg = [NSImageSymbolConfiguration
        configurationWithPointSize:11.0 weight:NSFontWeightRegular];
    NSImage* lock_img = [[NSImage imageWithSystemSymbolName:@"lock.fill"
                                      accessibilityDescription:@"Secure"]
        imageWithSymbolConfiguration:cfg];

    s.lock_icon = [[NSImageView alloc]
        initWithFrame:NSMakeRect(kAddrHPad, (kAddrHeight - 14.0) / 2.0, 14.0, 14.0)];
    [s.lock_icon setImage:lock_img];
    [s.lock_icon setContentTintColor:AegisLockIcon()];
    [s.lock_icon setAutoresizingMask:NSViewMaxXMargin];
    [s.address_container addSubview:s.lock_icon];
  }

  // Address text field — vertically centered in 32px container
  const CGFloat field_x = kAddrHPad + 14.0 + 6.0;  // after lock icon + gap
  const CGFloat field_w = addr_w - field_x - kAddrHPad;
  const CGFloat field_h = 20.0;
  const CGFloat field_y = (kAddrHeight - field_h) / 2.0;
  s.address_field = [[AegisAddressField alloc]
      initWithFrame:NSMakeRect(field_x, field_y, field_w, field_h)];
  s.address_field.addressContainer = s.address_container;
  [s.address_field setAutoresizingMask:NSViewWidthSizable];
  [s.address_field setBezeled:NO];
  [s.address_field setBordered:NO];
  [s.address_field setFocusRingType:NSFocusRingTypeNone];
  [s.address_field setDrawsBackground:NO];
  [s.address_field setBackgroundColor:[NSColor clearColor]];
  [s.address_field setTextColor:AegisPrimaryText()];
  [s.address_field setFont:[NSFont systemFontOfSize:13.0 weight:NSFontWeightRegular]];
  [s.address_field setPlaceholderString:@"Search or enter address"];

  // Style the placeholder
  {
    NSAttributedString* placeholder = [[NSAttributedString alloc]
        initWithString:@"Search or enter address"
            attributes:@{
              NSForegroundColorAttributeName : AegisPlaceholderText(),
              NSFontAttributeName : [NSFont systemFontOfSize:13.0 weight:NSFontWeightRegular]
            }];
    [s.address_field setPlaceholderAttributedString:placeholder];
  }

  [s.address_field setLineBreakMode:NSLineBreakByTruncatingTail];
  [s.address_field setUsesSingleLineMode:YES];
  [s.address_container addSubview:s.address_field];

  // ════════════════════════════════════════════════════════════════════════
  // WEB CONTENT CONTAINER
  // ════════════════════════════════════════════════════════════════════════

  s.web_container_view = [[NSView alloc]
      initWithFrame:NSMakeRect(0, 0, w, h - kToolbarHeight)];
  [s.web_container_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  [s.root_view addSubview:s.web_container_view];

  // ════════════════════════════════════════════════════════════════════════
  // WIRE UP CONTROLLER
  // ════════════════════════════════════════════════════════════════════════

  s.controller = [[AegisBrowserWindowController alloc] initWithStatePointer:&s];
  [s.window setDelegate:s.controller];

  [s.back_button setTarget:s.controller];
  [s.back_button setAction:@selector(navigateBack:)];
  [s.forward_button setTarget:s.controller];
  [s.forward_button setAction:@selector(navigateForward:)];
  [s.reload_button setTarget:s.controller];
  [s.reload_button setAction:@selector(reloadPage:)];
  [s.address_field setTarget:s.controller];
  [s.address_field setAction:@selector(submitLocation:)];
  [s.address_field setDelegate:s.controller];

  [s.controller updateAddress:[NSString stringWithUTF8String:title.c_str()]];
  [s.controller updateNavigationButtonsWithCanGoBack:NO
                                          canGoForward:NO
                                             isLoading:NO];
}

}  // namespace

// ═══════════════════════════════════════════════════════════════════════════════
// CONTROLLER IMPLEMENTATION
// ═══════════════════════════════════════════════════════════════════════════════

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

// ── Navigation actions ───────────────────────────────────────────────────────

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

// ── Address bar ──────────────────────────────────────────────────────────────

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
  [self unfocusAddressBar];
}

- (void)updateAddress:(NSString*)address {
  auto* state = [self state];
  if (address == nil || state->address_field == nil) {
    return;
  }
  if (![[state->address_field currentEditor] isKindOfClass:[NSTextView class]]) {
    [state->address_field setStringValue:address];
  }

  // Update lock icon visibility based on https
  if (state->lock_icon) {
    BOOL is_secure = [address hasPrefix:@"https://"];
    BOOL is_empty = (address.length == 0);
    state->lock_icon.hidden = (!is_secure && !is_empty) || is_empty;
  }
}

- (void)updateTitle:(NSString*)title {
  auto* state = [self state];
  if (title == nil) return;

  [state->window setTitle:title];
  if (state->tab_title_label) {
    [state->tab_title_label setStringValue:title];

    // Resize tab to fit title (with constraints).
    NSDictionary* attrs = @{
      NSFontAttributeName : state->tab_title_label.font
    };
    CGFloat text_w = [title sizeWithAttributes:attrs].width;
    CGFloat tab_w = text_w + kTabHPad * 2.0 + 4.0;
    tab_w = fmax(80.0, fmin(tab_w, 240.0));
    NSRect tf = state->tab_bg.frame;
    tf.size.width = tab_w;
    [state->tab_bg setFrame:tf];

    // Reposition new-tab button.
    if (state->new_tab_button) {
      NSRect ntf = state->new_tab_button.frame;
      ntf.origin.x = tf.origin.x + tf.size.width + 6.0;
      [state->new_tab_button setFrame:ntf];
    }
  }
}

// ── Navigation state ─────────────────────────────────────────────────────────

- (void)updateNavigationButtonsWithCanGoBack:(BOOL)canGoBack
                                 canGoForward:(BOOL)canGoForward
                                    isLoading:(BOOL)isLoading {
  auto* state = [self state];
  [state->back_button setEnabled:canGoBack];
  [state->forward_button setEnabled:canGoForward];

  // Switch reload ↔ stop icon.
  if (isLoading) {
    [state->reload_button setSymbolName:@"xmark"];
  } else {
    [state->reload_button setSymbolName:@"arrow.clockwise"];
  }

  // Progress bar.
  if (state->progress_view) {
    if (isLoading) {
      CGFloat toolbar_w = state->toolbar_view.frame.size.width;
      state->progress_view.hidden = NO;
      [state->progress_view setFrame:NSMakeRect(0, kSeparatorHeight, 0, kProgressHeight)];

      [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
        ctx.duration = 8.0;
        ctx.timingFunction = [CAMediaTimingFunction
            functionWithName:kCAMediaTimingFunctionEaseOut];
        [[state->progress_view animator]
            setFrame:NSMakeRect(0, kSeparatorHeight,
                                toolbar_w * 0.75, kProgressHeight)];
      } completionHandler:nil];
    } else {
      CGFloat toolbar_w = state->toolbar_view.frame.size.width;
      [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
        ctx.duration = 0.2;
        [[state->progress_view animator]
            setFrame:NSMakeRect(0, kSeparatorHeight,
                                toolbar_w, kProgressHeight)];
      } completionHandler:^{
        [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
          ctx.duration = 0.3;
          [[state->progress_view animator] setAlphaValue:0.0];
        } completionHandler:^{
          state->progress_view.hidden = YES;
          [state->progress_view setAlphaValue:1.0];
        }];
      }];
    }
  }
}

// ── Focus management ─────────────────────────────────────────────────────────

- (void)unfocusAddressBar {
  auto* state = [self state];
  [state->window makeFirstResponder:nil];
  if (state->address_container) {
    [state->address_container setFocused:NO];
  }
}

- (void)controlTextDidEndEditing:(NSNotification*)notification {
  (void)notification;
  auto* state = [self state];
  if (state->address_container) {
    [state->address_container setFocused:NO];
  }
}

// ── Window delegate ──────────────────────────────────────────────────────────

- (BOOL)windowShouldClose:(NSWindow*)window {
  (void)window;
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

// ═══════════════════════════════════════════════════════════════════════════════
// PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════════

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
  [state.controller updateTitle:value];
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
  state.root_view = nil;
  state.toolbar_view = nil;
  state.separator_view = nil;
  state.tab_bg = nil;
  state.tab_title_label = nil;
  state.new_tab_button = nil;
  state.back_button = nil;
  state.forward_button = nil;
  state.reload_button = nil;
  state.address_container = nil;
  state.lock_icon = nil;
  state.address_field = nil;
  state.progress_view = nil;
  state.web_container_view = nil;
  state.controller = nil;
  [NSApp terminate:nil];
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
