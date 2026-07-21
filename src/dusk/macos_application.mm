#include "macos_application.hpp"

#import <AppKit/AppKit.h>

namespace dusk {

void ConfigureMacOSHeadlessLaunch() {
    NSUserDefaults* defaults = [NSUserDefaults standardUserDefaults];
    NSMutableDictionary* arguments =
        [[defaults volatileDomainForName:NSArgumentDomain] mutableCopy];
    if (arguments == nil) {
        arguments = [[NSMutableDictionary alloc] init];
    }
    arguments[@"ApplePersistenceIgnoreState"] = @YES;
    arguments[@"NSQuitAlwaysKeepsWindows"] = @NO;
    [defaults setVolatileDomain:arguments forName:NSArgumentDomain];
}

}  // namespace dusk
