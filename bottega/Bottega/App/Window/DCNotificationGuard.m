// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
#import "DCNotificationGuard.h"
#import <objc/runtime.h>
#import <os/log.h>

/// Logged under the app's bundle id, the subsystem `AppInfo.logSubsystem` gives the Swift
/// side, so each build's entries stay apart from another build's in `log stream`.
static os_log_t DCNotificationLog(void) {
    static os_log_t log;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        log = os_log_create(NSBundle.mainBundle.bundleIdentifier.UTF8String ?: "me.studiolo.bottega", "notification");
    });
    return log;
}

/// The raising observer's first frames, so the next report names it.
static NSString *DCNotificationOrigin(NSException *exception) {
    NSArray<NSString *> *frames = exception.callStackSymbols;
    NSUInteger count = MIN(frames.count, (NSUInteger)12);
    return count == 0 ? @"" : [[frames subarrayWithRange:NSMakeRange(0, count)] componentsJoinedByString:@"\n"];
}

static void DCNotificationRaised(NSNotificationName name, NSException *exception) {
    os_log_error(DCNotificationLog(), "An observer raised while %{public}@ was posted, dropped: %{public}@ %{public}@\n%{public}@",
                 name, exception.name, exception.reason ?: @"", DCNotificationOrigin(exception));
}

static IMP DCPostWithUserInfoOriginal;
static IMP DCPostObjectOriginal;

static void DCPostWithUserInfoGuarded(id self, SEL _cmd, NSNotificationName name, id object, NSDictionary *userInfo) {
    @try {
        ((void (*)(id, SEL, NSNotificationName, id, NSDictionary *))DCPostWithUserInfoOriginal)(self, _cmd, name, object, userInfo);
    } @catch (NSException *exception) {
        DCNotificationRaised(name, exception);
    }
}

static void DCPostObjectGuarded(id self, SEL _cmd, NSNotificationName name, id object) {
    @try {
        ((void (*)(id, SEL, NSNotificationName, id))DCPostObjectOriginal)(self, _cmd, name, object);
    } @catch (NSException *exception) {
        DCNotificationRaised(name, exception);
    }
}

void DCInstallNotificationGuard(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        Class center = [NSNotificationCenter class];
        Method withUserInfo = class_getInstanceMethod(center, @selector(postNotificationName:object:userInfo:));
        if (withUserInfo != NULL) {
            DCPostWithUserInfoOriginal = method_setImplementation(withUserInfo, (IMP)DCPostWithUserInfoGuarded);
        }
        Method plain = class_getInstanceMethod(center, @selector(postNotificationName:object:));
        if (plain != NULL) {
            DCPostObjectOriginal = method_setImplementation(plain, (IMP)DCPostObjectGuarded);
        }
    });
}
