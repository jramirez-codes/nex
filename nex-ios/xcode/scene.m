// The UIScene life cycle, which apps built with the iOS 27 SDK must adopt or UIKit stops them
// at launch. winit 0.30 predates scenes: it makes its UIWindow with initWithFrame: during
// didFinishLaunching, before iOS connects the app's scene, and such a window is never shown.
// So nex's window is held back when winit first shows it, and attached to the scene once the
// scene connects (Info.plist names NexSceneDelegate in UIApplicationSceneManifest).
//
// winit keeps working otherwise: it follows the app through UIApplication's notifications
// (did become active, did enter background...), which UIKit still posts with scenes.
#import <UIKit/UIKit.h>
#import <objc/runtime.h>

static UIWindowScene *nex_scene;
static NSMutableArray<UIWindow *> *nex_waiting;

@interface NexSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property(strong, nonatomic) UIWindow *window;
@end

@implementation NexSceneDelegate
- (void)scene:(UIScene *)scene
    willConnectToSession:(UISceneSession *)session
                 options:(UISceneConnectionOptions *)options {
    if (![scene isKindOfClass:[UIWindowScene class]]) {
        return;
    }
    nex_scene = (UIWindowScene *)scene;
    for (UIWindow *window in nex_waiting) {
        window.windowScene = nex_scene;
        window.frame = nex_scene.coordinateSpace.bounds;
        self.window = window;
        [window makeKeyAndVisible];
    }
    [nex_waiting removeAllObjects];
}
@end

// UIWindow's own makeKeyAndVisible, which nex_make_key_and_visible wraps.
static void (*nex_original_make_key_and_visible)(UIWindow *, SEL);

static void nex_make_key_and_visible(UIWindow *self, SEL _cmd) {
    if (self.windowScene == nil) {
        if (nex_scene == nil) {
            // Too early: show it when the scene connects.
            if (![nex_waiting containsObject:self]) {
                [nex_waiting addObject:self];
            }
            return;
        }
        self.windowScene = nex_scene;
    }
    nex_original_make_key_and_visible(self, _cmd);
}

__attribute__((constructor)) static void nex_install_scene_support(void) {
    nex_waiting = [NSMutableArray array];
    Method method = class_getInstanceMethod([UIWindow class], @selector(makeKeyAndVisible));
    nex_original_make_key_and_visible =
        (void (*)(UIWindow *, SEL))method_setImplementation(method, (IMP)nex_make_key_and_visible);
}
