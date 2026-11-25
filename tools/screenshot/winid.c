// Print the CGWindowID of the first on-screen window owned by a process whose
// name matches argv[1] (substring) and, optionally, whose title contains argv[2].
// Used with `screencapture -l <id>` to capture ONE application window only.
#include <CoreGraphics/CoreGraphics.h>
#include <stdio.h>
#include <string.h>

static int contains(CFStringRef s, const char *needle) {
    if (!s || !needle) return needle == NULL;
    char buf[512];
    if (!CFStringGetCString(s, buf, sizeof buf, kCFStringEncodingUTF8)) return 0;
    return strstr(buf, needle) != NULL;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: winid <owner-substring> [title-substring]\n");
        return 2;
    }
    CFArrayRef list = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
                                                 kCGNullWindowID);
    if (!list) return 1;
    for (CFIndex i = 0; i < CFArrayGetCount(list); i++) {
        CFDictionaryRef w = CFArrayGetValueAtIndex(list, i);
        CFStringRef owner = CFDictionaryGetValue(w, kCGWindowOwnerName);
        CFStringRef title = CFDictionaryGetValue(w, kCGWindowName);
        CFNumberRef layer = CFDictionaryGetValue(w, kCGWindowLayer);
        int lv = 0;
        if (layer) CFNumberGetValue(layer, kCFNumberIntType, &lv);
        if (lv != 0 || !contains(owner, argv[1])) continue;
        if (argc > 2 && !contains(title, argv[2])) continue;
        CFNumberRef num = CFDictionaryGetValue(w, kCGWindowNumber);
        int id = 0;
        CFNumberGetValue(num, kCFNumberIntType, &id);
        printf("%d\n", id);
        CFRelease(list);
        return 0;
    }
    CFRelease(list);
    return 1;
}
