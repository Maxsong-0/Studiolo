#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

/// Runs the block and returns the Objective-C exception it raised, or nil when it
/// ran through. Swift cannot catch NSException on its own; AppKit raises one when a
/// view flag is set from inside the window's display cycle (see WindowHostingView).
NSException * _Nullable DCCatchException(void (NS_NOESCAPE ^block)(void));

NS_ASSUME_NONNULL_END
