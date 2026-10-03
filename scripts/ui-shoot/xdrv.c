/* An X11 pointer, keyboard and window driver, for driving tfg on a virtual
 * display. The Wayland path already has uimouse.py; this is the X11 twin, for
 * the Xvfb display where there is no compositor to route uinput.
 *
 * One process, one command per invocation, so the caller keeps control the way
 * uim.py does.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>
#include <X11/extensions/XTest.h>

static Display *dpy;
static Window root;

/* The window whose WM_CLASS or WM_NAME carries `needle`, else None. */
static Window find(Display *d, Window w, const char *needle, int depth) {
    char cls[256] = {0}, *nm = NULL;
    XClassHint ch = { 0 };
    if (XGetClassHint(d, w, &ch) == 0) ch.res_name = ch.res_class = NULL;
    if (ch.res_name) snprintf(cls, sizeof cls, "%s", ch.res_name);
    if (ch.res_class) {
        size_t n = strlen(cls);
        snprintf(cls + n, sizeof cls - n, " %s", ch.res_class);
    }
    XFetchName(d, w, &nm);
    const char *name = nm ? nm : "";
    int hit = needle && *needle &&
              (strstr(cls, needle) || strstr(name, needle));
    if (ch.res_name) XFree(ch.res_name);
    if (ch.res_class) XFree(ch.res_class);
    if (nm) XFree(nm);
    if (hit) return w;

    Window r, p, *kids = NULL;
    unsigned int n = 0;
    if (!XQueryTree(d, w, &r, &p, &kids, &n)) return None;
    Window found = None;
    for (unsigned int i = 0; i < n && found == None; i++)
        found = find(d, kids[i], needle, depth + 1);
    if (kids) XFree(kids);
    return found;
}

static void show(Window w, const char *tag) {
    if (w == None) { printf("%s: none\n", tag); return; }
    XWindowAttributes a;
    XGetWindowAttributes(dpy, w, &a);
    char cls[256] = {0}, *nm = NULL;
    XClassHint ch = { 0 };
    if (XGetClassHint(dpy, w, &ch) == 0) ch.res_name = ch.res_class = NULL;
    if (ch.res_class) snprintf(cls, sizeof cls, "%s", ch.res_class);
    XFetchName(dpy, w, &nm);
    const char *name = nm ? nm : "";
    printf("%s: 0x%lx class=%s name=%s at %d,%d size %dx%d\n",
           tag, w, cls, name, a.x, a.y, a.width, a.height);
    if (ch.res_name) XFree(ch.res_name);
    if (ch.res_class) XFree(ch.res_class);
    if (nm) XFree(nm);
}

static void click(int x, int y, int button) {
    XTestFakeMotionEvent(dpy, -1, x, y, CurrentTime);
    XFlush(dpy);
    usleep(40000);
    XTestFakeButtonEvent(dpy, button, True, CurrentTime);
    XFlush(dpy);
    /* A held press, not a zero-length one. Press and release delivered in the
     * same batch are dropped often enough to matter: a click on the island's
     * name field did nothing three runs in a row, and the same click with a
     * 70ms hold works every time. */
    usleep(70000);
    XTestFakeButtonEvent(dpy, button, False, CurrentTime);
    XFlush(dpy);
    usleep(40000);
}

/* XTest takes KEYCODES, so both the symbol and the modifier have to be
 * translated. Passing the Shift mask straight through looks right and is a
 * BadValue, because mask bit 0 is not a keycode. */
static void tap(KeySym ks, unsigned mods) {
    KeyCode kc = XKeysymToKeycode(dpy, ks);
    if (kc == 0) return;
    KeyCode ctrl = XKeysymToKeycode(dpy, XK_Control_L);
    KeyCode alt = XKeysymToKeycode(dpy, XK_Alt_L);
    if ((mods & ControlMask) && ctrl) XTestFakeKeyEvent(dpy, ctrl, True, CurrentTime);
    if ((mods & Mod1Mask) && alt) XTestFakeKeyEvent(dpy, alt, True, CurrentTime);
    KeyCode shift = XKeysymToKeycode(dpy, XK_Shift_L);
    if ((mods & ShiftMask) && shift) XTestFakeKeyEvent(dpy, shift, True, CurrentTime);
    XTestFakeKeyEvent(dpy, kc, True, CurrentTime);
    XTestFakeKeyEvent(dpy, kc, False, CurrentTime);
    if ((mods & ShiftMask) && shift) XTestFakeKeyEvent(dpy, shift, False, CurrentTime);
    if ((mods & Mod1Mask) && alt) XTestFakeKeyEvent(dpy, alt, False, CurrentTime);
    if ((mods & ControlMask) && ctrl) XTestFakeKeyEvent(dpy, ctrl, False, CurrentTime);
    XFlush(dpy);
}

/* ASCII to keysym. Latin-1 keysyms ARE the character code, so printable ASCII
 * needs no name lookup at all — and that matters, because XStringToKeysym("-")
 * returns NoSymbol (it wants a keysym NAME like "minus"), which cost a login:
 * the password field showed sixteen dots and the server said unauthorized,
 * because both hyphens had been silently dropped on the way in. */
static void type_ascii(const char *s) {
    for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
        unsigned c = *p;
        if (c == '\n') { tap(XK_Return, 0); continue; }
        if (c == '\t') { tap(XK_Tab, 0); continue; }
        if (c < 0x20 || c > 0x7e) continue;
        tap((KeySym)c, (c >= 'A' && c <= 'Z') ? ShiftMask : 0);
        usleep(12000);
    }
}

int main(int argc, char **argv) {
    const char *disp = getenv("XDRV_DISPLAY");
    dpy = XOpenDisplay(disp && *disp ? disp : NULL);
    if (!dpy) { fprintf(stderr, "cannot open display\n"); return 1; }
    root = DefaultRootWindow(dpy);

    if (argc < 2) { fprintf(stderr, "no command\n"); return 1; }
    const char *cmd = argv[1];
    int rc = 0;

    if (!strcmp(cmd, "list")) {
        Window w = find(dpy, root, "egui", 0);
        show(w, "egui");
        Window t = find(dpy, root, "tfg", 0);
        show(t, "tfg");
    } else if (!strcmp(cmd, "place") && argc >= 6) {
        Window w = find(dpy, root, "egui", 0);
        if (w == None) { fprintf(stderr, "no egui window\n"); return 2; }
        int x = atoi(argv[2]), y = atoi(argv[3]);
        unsigned wpx = (unsigned)atoi(argv[4]), hpx = (unsigned)atoi(argv[5]);
        XMoveResizeWindow(dpy, w, x, y, wpx, hpx);
        XFlush(dpy);
        usleep(400000);
        show(w, "placed");
    } else if (!strcmp(cmd, "move") && argc >= 4) {
        Window w = find(dpy, root, "egui", 0);
        XMoveWindow(dpy, w, atoi(argv[2]), atoi(argv[3]));
        XFlush(dpy);
    } else if (!strcmp(cmd, "click") && argc >= 4) {
        click(atoi(argv[2]), atoi(argv[3]), 1);
    } else if (!strcmp(cmd, "rclick") && argc >= 4) {
        click(atoi(argv[2]), atoi(argv[3]), 3);
    } else if (!strcmp(cmd, "mousemove") && argc >= 4) {
        XTestFakeMotionEvent(dpy, -1, atoi(argv[2]), atoi(argv[3]), CurrentTime);
        XFlush(dpy);
    } else if (!strcmp(cmd, "drag") && argc >= 5) {
        int x1 = atoi(argv[2]), y1 = atoi(argv[3]);
        int x2 = atoi(argv[4]), y2 = atoi(argv[5]);
        XTestFakeMotionEvent(dpy, -1, x1, y1, CurrentTime);
        XFlush(dpy); usleep(80000);
        XTestFakeButtonEvent(dpy, 1, True, CurrentTime);
        XFlush(dpy); usleep(80000);
        for (int i = 1; i <= 12; i++) {
            XTestFakeMotionEvent(dpy, -1,
                x1 + (x2 - x1) * i / 12, y1 + (y2 - y1) * i / 12, CurrentTime);
            XFlush(dpy); usleep(25000);
        }
        usleep(80000);
        XTestFakeButtonEvent(dpy, 1, False, CurrentTime);
        XFlush(dpy);
    } else if (!strcmp(cmd, "wheel") && argc >= 3) {
        /* Buttons 4 and 5 are the X11 wheel convention. Positive N scrolls
         * down, which is what "away from the operator" means on the wheel. */
        int n = atoi(argv[2]);
        int button = n > 0 ? 5 : 4;
        for (int i = 0; i < (n > 0 ? n : -n); i++) {
            XTestFakeButtonEvent(dpy, button, True, CurrentTime);
            XTestFakeButtonEvent(dpy, button, False, CurrentTime);
            XFlush(dpy);
            usleep(30000);
        }
    } else if (!strcmp(cmd, "type") && argc >= 3) {
        type_ascii(argv[2]);
    } else if (!strcmp(cmd, "key") && argc >= 3) {
        /* "ctrl+a", "shift+Tab": XStringToKeysym wants a key NAME, so the
         * modifier half is split off and the rest still resolves. Chords are
         * how a field gets cleared without counting backspaces. */
        char buf[64];
        snprintf(buf, sizeof buf, "%s", argv[2]);
        char *plus = strchr(buf, '+');
        unsigned mods = 0;
        KeySym ks;
        if (plus) {
            *plus = 0;
            for (char *m = buf; m < plus; m += 1) {
                switch (*m) {
                case 'c': case 'C': mods |= ControlMask; break;
                case 's': case 'S': mods |= ShiftMask; break;
                case 'a': case 'A': mods |= Mod1Mask; break;
                default: break;
                }
            }
            ks = XStringToKeysym(plus + 1);
        } else {
            ks = XStringToKeysym(buf);
        }
        if (ks == NoSymbol) { fprintf(stderr, "no such key: %s\n", argv[2]); rc = 2; }
        else tap(ks, mods);
    } else if (!strcmp(cmd, "focus")) {
        Window w = find(dpy, root, "egui", 0);
        if (w == None) { fprintf(stderr, "no egui window\n"); return 2; }
        XRaiseWindow(dpy, w);
        XSetInputFocus(dpy, w, RevertToParent, CurrentTime);
        XFlush(dpy);
        show(w, "focused");
    } else {
        fprintf(stderr, "unknown command: %s\n", cmd);
        rc = 2;
    }
    XCloseDisplay(dpy);
    return rc;
}