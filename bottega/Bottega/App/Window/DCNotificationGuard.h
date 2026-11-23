// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

/// A net under notification delivery.
///
/// A framework observer can raise while a notification is being posted, and the raise then
/// travels out of the post itself. On macOS 27 the ViewBridge observer of a popover's window
/// ordering raises this way (`-[NSRemoteView containingWindowWillOrderOnScreen:]`, reached from
/// `-[NSWindow _doWindowWillBeVisibleAsSheet:]` posting `NSWindowWillBeVisibleAsSheet`), and
/// because that happens inside a SwiftUI layout pass AppKit's layout block catches the raise and
/// ends the app with `+[NSApplication _crashOnException:]`; the Sept 18 2026 reports from the
/// release-notes cards on Settings > About read exactly so.
///
/// `DCInstallNotificationGuard()` wraps the center's two public post methods, so such a raise is
/// logged (subsystem the app's bundle id, category `notification`) and the post returns: the
/// window ordering and the popover finish their work. It is a net, not the cure: whatever raised
/// has still not seen its notification.
void DCInstallNotificationGuard(void);

NS_ASSUME_NONNULL_END
