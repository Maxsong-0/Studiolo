// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
#import <AppKit/AppKit.h>

NS_ASSUME_NONNULL_BEGIN

/// A window that survives asking for one display-cycle pass too many.
///
/// AppKit counts, per display cycle, how often a window is marked as needing another pass
/// of each kind (Display, Layout, Update Constraints, Update Structural Regions in Window).
/// Past a threshold it logs "The window has been marked as needing another %@ pass, but it
/// has already had %lu %@ passes"; once the count exceeds the number of views in the window
/// it raises that as an NSGenericException from the post itself, and uncaught it ends the
/// app. (Strings verified in the macOS 27.0 AppKit; the Sept 16 2026 reports raise from
/// `_postWindowNeedsUpdateConstraints` under `NSHostingView.setNeedsUpdate` and from
/// `_postWindowNeedsDisplay` under `NSClipView _immediateScrollToPoint:`, both beneath
/// `-[NSWindow _resetDragMargins]`.)
///
/// What runs the count up here: every window shows SwiftUI under a transparent title bar
/// (`fullSizeContentView`), so each pass ends with AppKit working out the drag region by
/// asking the hosting view for its opaque content, SwiftUI answers by evaluating the view
/// graph, and whatever that has pending, a `scrollTo` the timeline asked for on a geometry
/// change, a scroll view publishing a fresh transaction, dirties the window for another
/// pass, which asks again. The sequence settles on its own most days (the log line without
/// the exception is the sequence settling late); in a long conversation it can outrun the
/// view count first.
///
/// This class catches the raise at the window, where every view's request arrives, and
/// makes the same request again on the next run loop turn, where the count starts over.
/// The view keeps its own dirty flag throughout, so nothing is lost but a frame. It is a
/// net, not the cure: the sequence that runs the count up is still there, and a window
/// that needs the net logs it (subsystem the app's bundle id, category `window`), so a
/// storm can be seen. Guarding one view's flags (the earlier fix, on the hosting view) only
/// moved the raise to the next view down the chain, the scroll view's clip view.
@interface DCDisplayCycleGuardedWindow : NSWindow
@end

NS_ASSUME_NONNULL_END
