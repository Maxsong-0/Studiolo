// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
#import "DCDisplayCycleGuardedWindow.h"
#import <os/log.h>

/// AppKit's own entry points for a view telling its window it needs another pass. Private,
/// but unchanged since 10.x, and they are the exact frames that raise in the crash reports
/// (`-[NSWindow(NSDisplayCycle) _postWindowNeedsDisplay]` and friends). Declared here only so
/// `super` can be called; AppKit provides the implementations.
@interface NSWindow (DCDisplayCyclePosting)
- (void)_postWindowNeedsDisplay;
- (void)_postWindowNeedsLayout;
- (void)_postWindowNeedsUpdateConstraints;
- (void)_postWindowNeedsToResetDragMargins;
@end

@implementation DCDisplayCycleGuardedWindow

/// Logged under the app's bundle id, the subsystem `AppInfo.logSubsystem` gives the Swift
/// side, so each build's entries stay apart from another build's in `log stream`.
static os_log_t DCWindowLog(void) {
    static os_log_t log;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        log = os_log_create(NSBundle.mainBundle.bundleIdentifier.UTF8String ?: "me.studiolo.bottega", "window");
    });
    return log;
}

/// Runs `post`; when AppKit raises because this cycle has had more such passes than the
/// window has views, runs it once more on the next run loop turn, where the count starts
/// over. A second raise is logged and dropped: a fresh cycle that overruns at once is a
/// runaway this net cannot end.
- (void)dc_post:(void (^)(void))post {
    @try {
        post();
    } @catch (NSException *exception) {
        os_log(DCWindowLog(), "Display cycle pass limit hit, requesting again next turn: %{public}@ %{public}@",
               exception.name, exception.reason ?: @"");
        dispatch_async(dispatch_get_main_queue(), ^{
            @try {
                post();
            } @catch (NSException *again) {
                os_log_error(DCWindowLog(), "Display cycle pass limit hit twice in a row: %{public}@ %{public}@",
                             again.name, again.reason ?: @"");
            }
        });
    }
}

- (void)_postWindowNeedsDisplay {
    [self dc_post:^{ [super _postWindowNeedsDisplay]; }];
}

- (void)_postWindowNeedsLayout {
    [self dc_post:^{ [super _postWindowNeedsLayout]; }];
}

- (void)_postWindowNeedsUpdateConstraints {
    [self dc_post:^{ [super _postWindowNeedsUpdateConstraints]; }];
}

- (void)_postWindowNeedsToResetDragMargins {
    [self dc_post:^{ [super _postWindowNeedsToResetDragMargins]; }];
}

@end
