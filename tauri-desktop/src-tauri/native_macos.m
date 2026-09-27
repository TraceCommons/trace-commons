#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <ServiceManagement/ServiceManagement.h>
#import <UserNotifications/UserNotifications.h>
#import <dispatch/dispatch.h>

static const NSTimeInterval TC_TIMEOUT = 5.0;
static NSString * const TC_DIGEST_CATEGORY = @"trace-commons.digest";
static NSString * const TC_REVIEW_ACTION = @"trace-commons.review";
static NSString * const TC_NOT_NOW_ACTION = @"trace-commons.not-now";
static NSObject<UNUserNotificationCenterDelegate> *tc_notification_delegate = nil;

// Set once by tc_macos_notification_configure. Rust owns what a Review
// click does (show the window, open the review queue), so the click never
// goes through LaunchServices, where another app claiming
// tracecommons:// could receive it.
typedef void (*tc_review_callback)(void);
static tc_review_callback tc_on_review = NULL;

@interface TCNotificationDelegate : NSObject <UNUserNotificationCenterDelegate>
@end

@implementation TCNotificationDelegate
- (void)userNotificationCenter:(UNUserNotificationCenter *)center
       didReceiveNotificationResponse:(UNNotificationResponse *)response
                withCompletionHandler:(void (^)(void))completionHandler {
    (void)center;
    NSString *action = response.actionIdentifier;
    if ([action isEqualToString:TC_REVIEW_ACTION]
        || [action isEqualToString:UNNotificationDefaultActionIdentifier]) {
        tc_review_callback on_review = tc_on_review;
        if (on_review != NULL) {
            // The delegate's thread is not guaranteed; the window work the
            // callback triggers belongs on the main thread.
            dispatch_async(dispatch_get_main_queue(), ^{
                on_review();
            });
        }
    }
    completionHandler();
}
@end

static BOOL tc_has_application_bundle(void) {
    NSBundle *bundle = [NSBundle mainBundle];
    NSURL *bundle_url = bundle.bundleURL;
    NSString *bundle_identifier = bundle.bundleIdentifier;
    if (bundle_url == nil || bundle_identifier.length == 0) return NO;
    return [bundle_url.pathExtension caseInsensitiveCompare:@"app"] == NSOrderedSame;
}

static BOOL tc_available(void) {
    return tc_has_application_bundle()
        && NSClassFromString(@"UNUserNotificationCenter") != nil;
}

static UNUserNotificationCenter *tc_notification_center(void) {
    return [UNUserNotificationCenter currentNotificationCenter];
}

static int tc_notification_status_value(UNAuthorizationStatus status) {
    switch (status) {
        case UNAuthorizationStatusNotDetermined: return 0;
        case UNAuthorizationStatusDenied: return 1;
        case UNAuthorizationStatusAuthorized: return 2;
        case UNAuthorizationStatusProvisional: return 3;
        default: return -1;
    }
}

int tc_macos_notification_configure(tc_review_callback on_review) {
    @autoreleasepool {
        tc_on_review = on_review;
        if (!tc_available()) return -1;
        if (tc_notification_delegate == nil) {
            tc_notification_delegate = [TCNotificationDelegate new];
        }
        [tc_notification_center() setDelegate:tc_notification_delegate];
        UNNotificationAction *review = [UNNotificationAction
            actionWithIdentifier:TC_REVIEW_ACTION
            title:@"Review"
            options:UNNotificationActionOptionForeground];
        UNNotificationAction *not_now = [UNNotificationAction
            actionWithIdentifier:TC_NOT_NOW_ACTION
            title:@"Not now"
            options:0];
        UNNotificationCategory *category = [UNNotificationCategory
            categoryWithIdentifier:TC_DIGEST_CATEGORY
            actions:@[review, not_now]
            intentIdentifiers:@[]
            options:0];
        [tc_notification_center() setNotificationCategories:[NSSet setWithObject:category]];
        return 0;
    }
}

int tc_macos_notification_status(void) {
    @autoreleasepool {
        if (!tc_available()) return -1;
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block UNAuthorizationStatus status = UNAuthorizationStatusNotDetermined;
        [tc_notification_center() getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
            status = settings.authorizationStatus;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_time_t deadline = dispatch_time(DISPATCH_TIME_NOW, (int64_t)(TC_TIMEOUT * NSEC_PER_SEC));
        if (dispatch_semaphore_wait(semaphore, deadline) != 0) return -1;
        return tc_notification_status_value(status);
    }
}

int tc_macos_request_notification_permission(void) {
    @autoreleasepool {
        if (!tc_available()) return -1;
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block BOOL granted = NO;
        [tc_notification_center() requestAuthorizationWithOptions:(UNAuthorizationOptionAlert | UNAuthorizationOptionSound)
            completionHandler:^(BOOL did_grant, NSError *error) {
                granted = did_grant && error == nil;
                dispatch_semaphore_signal(semaphore);
            }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
        return granted ? 1 : 0;
    }
}

int tc_macos_post_digest(const char *body) {
    @autoreleasepool {
        if (!tc_available() || body == NULL) return -1;
        int permission = tc_macos_notification_status();
        if (permission != 2 && permission != 3) return 0;
        NSString *body_string = [NSString stringWithUTF8String:body];
        if (body_string == nil || body_string.length == 0 || body_string.length > 4000) return -1;
        UNMutableNotificationContent *content = [[UNMutableNotificationContent alloc] init];
        content.title = @"Trace Commons";
        content.body = body_string;
        content.categoryIdentifier = TC_DIGEST_CATEGORY;
        if (@available(macOS 12.0, *)) content.interruptionLevel = UNNotificationInterruptionLevelPassive;
        NSString *identifier = [[NSUUID UUID] UUIDString];
        UNNotificationRequest *request = [UNNotificationRequest
            requestWithIdentifier:identifier
            content:content
            trigger:nil];
        [tc_notification_center() addNotificationRequest:request withCompletionHandler:nil];
        return 0;
    }
}

static int tc_macos_login_status(void) {
    if (tc_has_application_bundle() && @available(macOS 13.0, *)) {
        return (int)[[SMAppService mainAppService] status];
    }
    return 3;
}

int tc_macos_login_item_status(void) {
    @autoreleasepool {
        return tc_macos_login_status();
    }
}

int tc_macos_set_login_item(int enabled) {
    @autoreleasepool {
        if (tc_has_application_bundle() && @available(macOS 13.0, *)) {
            SMAppService *service = [SMAppService mainAppService];
            NSError *error = nil;
            BOOL ok = enabled
                ? [service registerAndReturnError:&error]
                : [service unregisterAndReturnError:&error];
            if (!ok || error != nil) return -1;
            return tc_macos_login_status();
        }
        return 3;
    }
}
