// usbtrace: log every IOUSBLib control/bulk transfer a process makes, as a
// USBPcap-format pcap (link type 249, readable by `ubmdf replay-check` and
// Wireshark) plus a text log. macOS only; inject with DYLD_INSERT_LIBRARIES
// into a binary without the hardened runtime (e.g. an ad-hoc re-signed copy).
//
//   USBTRACE_PCAP=/path/out.pcap USBTRACE_LOG=/path/out.txt \
//   DYLD_INSERT_LIBRARIES=/path/libusbtrace.dylib <program> ...
//
// Hooks IOCreatePlugInInterfaceForService, then QueryInterface on the returned
// plug-in, then swaps the IOUSBDeviceInterface / IOUSBInterfaceInterface vtable
// for a patched copy. Wrapped: DeviceRequest[TO][Async[TO]], ControlRequest[TO]
// [Async[TO]], WritePipe[TO][Async[TO]], ReadPipe[TO][Async[TO]], ResetDevice,
// USBDeviceReEnumerate, ClearPipeStall[BothEnds], AbortPipe, ResetPipe.

#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOCFPlugIn.h>
#include <IOKit/usb/IOUSBLib.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>

typedef IOUSBDeviceInterface942 DevVT;
typedef IOUSBInterfaceInterface942 IfVT;

static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;
static FILE *g_pcap, *g_log;
static uint64_t g_irp = 1;

// ---- output -------------------------------------------------------------

static void open_outputs(void) {
    if (g_pcap || g_log) return;
    const char *p = getenv("USBTRACE_PCAP");
    const char *l = getenv("USBTRACE_LOG");
    g_pcap = fopen(p ? p : "/tmp/usbtrace.pcap", "wb");
    g_log = fopen(l ? l : "/tmp/usbtrace.txt", "w");
    if (g_pcap) {
        uint32_t magic = 0xa1b2c3d4, snap = 0x00ffffff, lt = 249, zero = 0;
        uint16_t vmaj = 2, vmin = 4;
        fwrite(&magic, 4, 1, g_pcap);
        fwrite(&vmaj, 2, 1, g_pcap);
        fwrite(&vmin, 2, 1, g_pcap);
        fwrite(&zero, 4, 1, g_pcap);
        fwrite(&zero, 4, 1, g_pcap);
        fwrite(&snap, 4, 1, g_pcap);
        fwrite(&lt, 4, 1, g_pcap);
        fflush(g_pcap);
    }
}

static void logf_(const char *fmt, ...) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    pthread_mutex_lock(&g_lock);
    open_outputs();
    if (g_log) {
        fprintf(g_log, "%ld.%06d ", (long)tv.tv_sec, (int)tv.tv_usec);
        va_list ap;
        va_start(ap, fmt);
        vfprintf(g_log, fmt, ap);
        va_end(ap);
        fputc('\n', g_log);
        fflush(g_log);
    }
    pthread_mutex_unlock(&g_lock);
}

static void hexs(char *out, size_t cap, const void *d, size_t n) {
    const uint8_t *b = d;
    size_t i, o = 0;
    for (i = 0; i < n && o + 3 < cap; i++) o += (size_t)snprintf(out + o, cap - o, "%02x", b[i]);
    if (i < n && o + 4 < cap) snprintf(out + o, cap - o, "...");
    if (n == 0 && cap) out[0] = 0;
}

// One USBPcap record. stage < 0 = no stage byte (bulk).
static void pcap_rec(uint64_t irp, int completion, uint32_t status, uint8_t ep, uint8_t xfer, int stage,
                     const void *payload, uint32_t plen, uint32_t length) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    uint8_t h[28];
    uint16_t hl = stage >= 0 ? 28 : 27;
    memset(h, 0, sizeof h);
    memcpy(h + 0, &hl, 2);
    memcpy(h + 2, &irp, 8);
    memcpy(h + 10, &status, 4);
    h[16] = completion ? 1 : 0;
    uint16_t bus = 1, dev = 1;
    memcpy(h + 17, &bus, 2);
    memcpy(h + 19, &dev, 2);
    h[21] = ep;
    h[22] = xfer;
    memcpy(h + 23, &length, 4);
    if (stage >= 0) h[27] = (uint8_t)stage;
    uint32_t sec = (uint32_t)tv.tv_sec, usec = (uint32_t)tv.tv_usec, incl = hl + plen;
    pthread_mutex_lock(&g_lock);
    open_outputs();
    if (g_pcap) {
        fwrite(&sec, 4, 1, g_pcap);
        fwrite(&usec, 4, 1, g_pcap);
        fwrite(&incl, 4, 1, g_pcap);
        fwrite(&incl, 4, 1, g_pcap);
        fwrite(h, hl, 1, g_pcap);
        if (plen) fwrite(payload, plen, 1, g_pcap);
        fflush(g_pcap);
    }
    pthread_mutex_unlock(&g_lock);
}

static uint64_t next_irp(void) { return __sync_fetch_and_add(&g_irp, 1); }

// Control transfer, submit side. Returns the IRP id for the completion record.
static uint64_t ctl_submit(const char *who, uint8_t rt, uint8_t rq, uint16_t val, uint16_t idx, uint16_t len,
                           const void *data) {
    uint64_t irp = next_irp();
    uint8_t buf[8 + 4096];
    buf[0] = rt;
    buf[1] = rq;
    memcpy(buf + 2, &val, 2);
    memcpy(buf + 4, &idx, 2);
    memcpy(buf + 6, &len, 2);
    uint32_t extra = 0;
    if (!(rt & 0x80) && len && data) {
        extra = len > 4096 ? 4096 : len;
        memcpy(buf + 8, data, extra);
    }
    pcap_rec(irp, 0, 0, (rt & 0x80) ? 0x80 : 0x00, 2, 0, buf, 8 + extra, 8 + extra);
    char hx[200];
    hexs(hx, sizeof hx, buf + 8, extra);
    logf_("%s SETUP %02x %02x wValue=%04x wIndex=%04x wLength=%u %s", who, rt, rq, val, idx, len, hx);
    return irp;
}

static void ctl_complete(const char *who, uint64_t irp, uint8_t rt, IOReturn kr, const void *data, uint32_t done) {
    uint32_t n = (rt & 0x80) && data ? done : 0;
    pcap_rec(irp, 1, (uint32_t)kr, (rt & 0x80) ? 0x80 : 0x00, 2, 3, data, n, done);
    char hx[200];
    hexs(hx, sizeof hx, data, n);
    logf_("%s DONE kr=0x%08x len=%u %s", who, kr, done, hx);
}

// ---- vtable bookkeeping ---------------------------------------------------

#define MAXOBJ 64
static struct {
    void *copy;
    void *orig;
} g_vt[MAXOBJ];
static int g_nvt;

static void *orig_of(void *self) {
    void *vt = *(void **)self;
    for (int i = 0; i < g_nvt; i++)
        if (g_vt[i].copy == vt) return g_vt[i].orig;
    return vt; // not ours (should not happen)
}

#define DEV(self) ((DevVT *)orig_of(self))
#define IFC(self) ((IfVT *)orig_of(self))

// ---- async completion trampolines -------------------------------------------

typedef struct {
    IOAsyncCallback1 cb;
    void *refcon;
    uint64_t irp;
    uint8_t rt;
    void *buf;
    uint8_t ep;
    uint8_t is_ctl;
    char who[24];
} AsyncCtx;

static void async_done(void *refcon, IOReturn result, void *arg0) {
    AsyncCtx *c = refcon;
    uint32_t n = (uint32_t)(uintptr_t)arg0;
    if (c->is_ctl) {
        ctl_complete(c->who, c->irp, c->rt, result, c->buf, n);
    } else {
        int in = c->ep & 0x80;
        pcap_rec(c->irp, 1, (uint32_t)result, c->ep, 3, -1, in ? c->buf : NULL, in ? n : 0, n);
        logf_("%s DONE ep=%02x kr=0x%08x bytes=%u", c->who, c->ep, result, n);
    }
    IOAsyncCallback1 cb = c->cb;
    void *rc = c->refcon;
    free(c);
    if (cb) cb(rc, result, arg0);
}

static AsyncCtx *mk_ctx(const char *who, IOAsyncCallback1 cb, void *refcon, uint64_t irp, uint8_t rt, void *buf,
                        uint8_t ep, uint8_t is_ctl) {
    AsyncCtx *c = calloc(1, sizeof *c);
    c->cb = cb;
    c->refcon = refcon;
    c->irp = irp;
    c->rt = rt;
    c->buf = buf;
    c->ep = ep;
    c->is_ctl = is_ctl;
    snprintf(c->who, sizeof c->who, "%s", who);
    return c;
}

// ---- device interface wrappers ---------------------------------------------

static IOReturn w_DeviceRequest(void *self, IOUSBDevRequest *r) {
    uint64_t irp = ctl_submit("dev.DeviceRequest", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    IOReturn kr = DEV(self)->DeviceRequest(self, r);
    ctl_complete("dev.DeviceRequest", irp, r->bmRequestType, kr, r->pData, r->wLenDone);
    return kr;
}
static IOReturn w_DeviceRequestTO(void *self, IOUSBDevRequestTO *r) {
    uint64_t irp = ctl_submit("dev.DeviceRequestTO", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    IOReturn kr = DEV(self)->DeviceRequestTO(self, r);
    ctl_complete("dev.DeviceRequestTO", irp, r->bmRequestType, kr, r->pData, r->wLenDone);
    return kr;
}
static IOReturn w_DeviceRequestAsync(void *self, IOUSBDevRequest *r, IOAsyncCallback1 cb, void *rc) {
    uint64_t irp = ctl_submit("dev.DeviceRequestAsync", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    AsyncCtx *c = mk_ctx("dev.DeviceRequestAsync", cb, rc, irp, r->bmRequestType, r->pData, 0, 1);
    IOReturn kr = DEV(self)->DeviceRequestAsync(self, r, async_done, c);
    if (kr != kIOReturnSuccess) { ctl_complete("dev.DeviceRequestAsync(submit)", irp, r->bmRequestType, kr, NULL, 0); free(c); }
    return kr;
}
static IOReturn w_DeviceRequestAsyncTO(void *self, IOUSBDevRequestTO *r, IOAsyncCallback1 cb, void *rc) {
    uint64_t irp = ctl_submit("dev.DeviceRequestAsyncTO", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    AsyncCtx *c = mk_ctx("dev.DeviceRequestAsyncTO", cb, rc, irp, r->bmRequestType, r->pData, 0, 1);
    IOReturn kr = DEV(self)->DeviceRequestAsyncTO(self, r, async_done, c);
    if (kr != kIOReturnSuccess) { ctl_complete("dev.DeviceRequestAsyncTO(submit)", irp, r->bmRequestType, kr, NULL, 0); free(c); }
    return kr;
}
static IOReturn w_ResetDevice(void *self) {
    logf_("dev.ResetDevice");
    IOReturn kr = DEV(self)->ResetDevice(self);
    logf_("dev.ResetDevice kr=0x%08x", kr);
    return kr;
}
static IOReturn w_USBDeviceReEnumerate(void *self, UInt32 opts) {
    logf_("dev.USBDeviceReEnumerate options=0x%x", opts);
    IOReturn kr = DEV(self)->USBDeviceReEnumerate(self, opts);
    logf_("dev.USBDeviceReEnumerate kr=0x%08x", kr);
    return kr;
}

// ---- interface wrappers ------------------------------------------------------

static uint8_t pipe_ep(void *self, UInt8 pipe) {
    UInt8 dir = 0, num = 0, type = 0, interval = 0;
    UInt16 mps = 0;
    if (pipe == 0) return 0;
    if (IFC(self)->GetPipeProperties(self, pipe, &dir, &num, &type, &mps, &interval) != kIOReturnSuccess) return 0xff;
    return (uint8_t)(num | (dir == kUSBIn ? 0x80 : 0));
}

static IOReturn w_ControlRequest(void *self, UInt8 pipe, IOUSBDevRequest *r) {
    uint64_t irp = ctl_submit("if.ControlRequest", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    IOReturn kr = IFC(self)->ControlRequest(self, pipe, r);
    ctl_complete("if.ControlRequest", irp, r->bmRequestType, kr, r->pData, r->wLenDone);
    return kr;
}
static IOReturn w_ControlRequestTO(void *self, UInt8 pipe, IOUSBDevRequestTO *r) {
    uint64_t irp = ctl_submit("if.ControlRequestTO", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    IOReturn kr = IFC(self)->ControlRequestTO(self, pipe, r);
    ctl_complete("if.ControlRequestTO", irp, r->bmRequestType, kr, r->pData, r->wLenDone);
    return kr;
}
static IOReturn w_ControlRequestAsync(void *self, UInt8 pipe, IOUSBDevRequest *r, IOAsyncCallback1 cb, void *rc) {
    uint64_t irp = ctl_submit("if.ControlRequestAsync", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    AsyncCtx *c = mk_ctx("if.ControlRequestAsync", cb, rc, irp, r->bmRequestType, r->pData, 0, 1);
    IOReturn kr = IFC(self)->ControlRequestAsync(self, pipe, r, async_done, c);
    if (kr != kIOReturnSuccess) { ctl_complete("if.ControlRequestAsync(submit)", irp, r->bmRequestType, kr, NULL, 0); free(c); }
    return kr;
}
static IOReturn w_ControlRequestAsyncTO(void *self, UInt8 pipe, IOUSBDevRequestTO *r, IOAsyncCallback1 cb, void *rc) {
    uint64_t irp = ctl_submit("if.ControlRequestAsyncTO", r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength, r->pData);
    AsyncCtx *c = mk_ctx("if.ControlRequestAsyncTO", cb, rc, irp, r->bmRequestType, r->pData, 0, 1);
    IOReturn kr = IFC(self)->ControlRequestAsyncTO(self, pipe, r, async_done, c);
    if (kr != kIOReturnSuccess) { ctl_complete("if.ControlRequestAsyncTO(submit)", irp, r->bmRequestType, kr, NULL, 0); free(c); }
    return kr;
}

static void bulk_out_submit(uint64_t irp, uint8_t ep, const void *buf, UInt32 size, const char *who) {
    pcap_rec(irp, 0, 0, ep, 3, -1, buf, size, size);
    logf_("%s ep=%02x bytes=%u", who, ep, size);
}

static IOReturn w_WritePipe(void *self, UInt8 pipe, void *buf, UInt32 size) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    bulk_out_submit(irp, ep, buf, size, "if.WritePipe");
    IOReturn kr = IFC(self)->WritePipe(self, pipe, buf, size);
    pcap_rec(irp, 1, (uint32_t)kr, ep, 3, -1, NULL, 0, size);
    if (kr) logf_("if.WritePipe kr=0x%08x", kr);
    return kr;
}
static IOReturn w_WritePipeTO(void *self, UInt8 pipe, void *buf, UInt32 size, UInt32 nd, UInt32 cd) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    bulk_out_submit(irp, ep, buf, size, "if.WritePipeTO");
    IOReturn kr = IFC(self)->WritePipeTO(self, pipe, buf, size, nd, cd);
    pcap_rec(irp, 1, (uint32_t)kr, ep, 3, -1, NULL, 0, size);
    if (kr) logf_("if.WritePipeTO kr=0x%08x", kr);
    return kr;
}
static IOReturn w_WritePipeAsync(void *self, UInt8 pipe, void *buf, UInt32 size, IOAsyncCallback1 cb, void *rc) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    bulk_out_submit(irp, ep, buf, size, "if.WritePipeAsync");
    AsyncCtx *c = mk_ctx("if.WritePipeAsync", cb, rc, irp, 0, buf, ep, 0);
    IOReturn kr = IFC(self)->WritePipeAsync(self, pipe, buf, size, async_done, c);
    if (kr != kIOReturnSuccess) { logf_("if.WritePipeAsync submit kr=0x%08x", kr); free(c); }
    return kr;
}
static IOReturn w_WritePipeAsyncTO(void *self, UInt8 pipe, void *buf, UInt32 size, UInt32 nd, UInt32 cd,
                                   IOAsyncCallback1 cb, void *rc) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    bulk_out_submit(irp, ep, buf, size, "if.WritePipeAsyncTO");
    AsyncCtx *c = mk_ctx("if.WritePipeAsyncTO", cb, rc, irp, 0, buf, ep, 0);
    IOReturn kr = IFC(self)->WritePipeAsyncTO(self, pipe, buf, size, nd, cd, async_done, c);
    if (kr != kIOReturnSuccess) { logf_("if.WritePipeAsyncTO submit kr=0x%08x", kr); free(c); }
    return kr;
}
static IOReturn w_ReadPipe(void *self, UInt8 pipe, void *buf, UInt32 *size) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    UInt32 req = *size;
    pcap_rec(irp, 0, 0, ep, 3, -1, NULL, 0, req);
    IOReturn kr = IFC(self)->ReadPipe(self, pipe, buf, size);
    pcap_rec(irp, 1, (uint32_t)kr, ep, 3, -1, buf, kr ? 0 : *size, *size);
    logf_("if.ReadPipe ep=%02x req=%u got=%u kr=0x%08x", ep, req, *size, kr);
    return kr;
}
static IOReturn w_ReadPipeTO(void *self, UInt8 pipe, void *buf, UInt32 *size, UInt32 nd, UInt32 cd) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    UInt32 req = *size;
    pcap_rec(irp, 0, 0, ep, 3, -1, NULL, 0, req);
    IOReturn kr = IFC(self)->ReadPipeTO(self, pipe, buf, size, nd, cd);
    pcap_rec(irp, 1, (uint32_t)kr, ep, 3, -1, buf, kr ? 0 : *size, *size);
    logf_("if.ReadPipeTO ep=%02x req=%u got=%u kr=0x%08x", ep, req, *size, kr);
    return kr;
}
static IOReturn w_ReadPipeAsync(void *self, UInt8 pipe, void *buf, UInt32 size, IOAsyncCallback1 cb, void *rc) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    pcap_rec(irp, 0, 0, ep, 3, -1, NULL, 0, size);
    logf_("if.ReadPipeAsync ep=%02x req=%u", ep, size);
    AsyncCtx *c = mk_ctx("if.ReadPipeAsync", cb, rc, irp, 0, buf, ep, 0);
    IOReturn kr = IFC(self)->ReadPipeAsync(self, pipe, buf, size, async_done, c);
    if (kr != kIOReturnSuccess) { logf_("if.ReadPipeAsync submit kr=0x%08x", kr); free(c); }
    return kr;
}
static IOReturn w_ReadPipeAsyncTO(void *self, UInt8 pipe, void *buf, UInt32 size, UInt32 nd, UInt32 cd,
                                  IOAsyncCallback1 cb, void *rc) {
    uint8_t ep = pipe_ep(self, pipe);
    uint64_t irp = next_irp();
    pcap_rec(irp, 0, 0, ep, 3, -1, NULL, 0, size);
    logf_("if.ReadPipeAsyncTO ep=%02x req=%u", ep, size);
    AsyncCtx *c = mk_ctx("if.ReadPipeAsyncTO", cb, rc, irp, 0, buf, ep, 0);
    IOReturn kr = IFC(self)->ReadPipeAsyncTO(self, pipe, buf, size, nd, cd, async_done, c);
    if (kr != kIOReturnSuccess) { logf_("if.ReadPipeAsyncTO submit kr=0x%08x", kr); free(c); }
    return kr;
}
static IOReturn w_ClearPipeStall(void *self, UInt8 pipe) {
    IOReturn kr = IFC(self)->ClearPipeStall(self, pipe);
    logf_("if.ClearPipeStall pipe=%u kr=0x%08x", pipe, kr);
    return kr;
}
static IOReturn w_ClearPipeStallBothEnds(void *self, UInt8 pipe) {
    IOReturn kr = IFC(self)->ClearPipeStallBothEnds(self, pipe);
    logf_("if.ClearPipeStallBothEnds pipe=%u kr=0x%08x", pipe, kr);
    return kr;
}
static IOReturn w_AbortPipe(void *self, UInt8 pipe) {
    IOReturn kr = IFC(self)->AbortPipe(self, pipe);
    logf_("if.AbortPipe pipe=%u kr=0x%08x", pipe, kr);
    return kr;
}
static IOReturn w_ResetPipe(void *self, UInt8 pipe) {
    IOReturn kr = IFC(self)->ResetPipe(self, pipe);
    logf_("if.ResetPipe pipe=%u kr=0x%08x", pipe, kr);
    return kr;
}

// ---- interface-ID tables: struct size per requested version -------------------

typedef struct {
    CFUUIDRef (*id)(void);
    size_t size;
    int is_dev;
    const char *name;
} VtKind;

#define U(x) static CFUUIDRef id_##x(void) { return x; }
U(kIOUSBDeviceInterfaceID) U(kIOUSBDeviceInterfaceID182) U(kIOUSBDeviceInterfaceID187) U(kIOUSBDeviceInterfaceID197)
U(kIOUSBDeviceInterfaceID245) U(kIOUSBDeviceInterfaceID300) U(kIOUSBDeviceInterfaceID320) U(kIOUSBDeviceInterfaceID500)
U(kIOUSBDeviceInterfaceID650) U(kIOUSBDeviceInterfaceID942)
U(kIOUSBInterfaceInterfaceID) U(kIOUSBInterfaceInterfaceID182) U(kIOUSBInterfaceInterfaceID183)
U(kIOUSBInterfaceInterfaceID190) U(kIOUSBInterfaceInterfaceID192) U(kIOUSBInterfaceInterfaceID197)
U(kIOUSBInterfaceInterfaceID220) U(kIOUSBInterfaceInterfaceID245) U(kIOUSBInterfaceInterfaceID300)
U(kIOUSBInterfaceInterfaceID500) U(kIOUSBInterfaceInterfaceID550) U(kIOUSBInterfaceInterfaceID650)
U(kIOUSBInterfaceInterfaceID700) U(kIOUSBInterfaceInterfaceID800) U(kIOUSBInterfaceInterfaceID942)

static const VtKind kinds[] = {
    {id_kIOUSBDeviceInterfaceID, sizeof(IOUSBDeviceInterface), 1, "Device"},
    {id_kIOUSBDeviceInterfaceID182, sizeof(IOUSBDeviceInterface182), 1, "Device182"},
    {id_kIOUSBDeviceInterfaceID187, sizeof(IOUSBDeviceInterface187), 1, "Device187"},
    {id_kIOUSBDeviceInterfaceID197, sizeof(IOUSBDeviceInterface197), 1, "Device197"},
    {id_kIOUSBDeviceInterfaceID245, sizeof(IOUSBDeviceInterface245), 1, "Device245"},
    {id_kIOUSBDeviceInterfaceID300, sizeof(IOUSBDeviceInterface300), 1, "Device300"},
    {id_kIOUSBDeviceInterfaceID320, sizeof(IOUSBDeviceInterface320), 1, "Device320"},
    {id_kIOUSBDeviceInterfaceID500, sizeof(IOUSBDeviceInterface500), 1, "Device500"},
    {id_kIOUSBDeviceInterfaceID650, sizeof(IOUSBDeviceInterface650), 1, "Device650"},
    {id_kIOUSBDeviceInterfaceID942, sizeof(IOUSBDeviceInterface942), 1, "Device942"},
    {id_kIOUSBInterfaceInterfaceID, sizeof(IOUSBInterfaceInterface), 0, "Interface"},
    {id_kIOUSBInterfaceInterfaceID182, sizeof(IOUSBInterfaceInterface182), 0, "Interface182"},
    {id_kIOUSBInterfaceInterfaceID183, sizeof(IOUSBInterfaceInterface183), 0, "Interface183"},
    {id_kIOUSBInterfaceInterfaceID190, sizeof(IOUSBInterfaceInterface190), 0, "Interface190"},
    {id_kIOUSBInterfaceInterfaceID192, sizeof(IOUSBInterfaceInterface192), 0, "Interface192"},
    {id_kIOUSBInterfaceInterfaceID197, sizeof(IOUSBInterfaceInterface197), 0, "Interface197"},
    {id_kIOUSBInterfaceInterfaceID220, sizeof(IOUSBInterfaceInterface220), 0, "Interface220"},
    {id_kIOUSBInterfaceInterfaceID245, sizeof(IOUSBInterfaceInterface245), 0, "Interface245"},
    {id_kIOUSBInterfaceInterfaceID300, sizeof(IOUSBInterfaceInterface300), 0, "Interface300"},
    {id_kIOUSBInterfaceInterfaceID500, sizeof(IOUSBInterfaceInterface500), 0, "Interface500"},
    {id_kIOUSBInterfaceInterfaceID550, sizeof(IOUSBInterfaceInterface550), 0, "Interface550"},
    {id_kIOUSBInterfaceInterfaceID650, sizeof(IOUSBInterfaceInterface650), 0, "Interface650"},
    {id_kIOUSBInterfaceInterfaceID700, sizeof(IOUSBInterfaceInterface700), 0, "Interface700"},
    {id_kIOUSBInterfaceInterfaceID800, sizeof(IOUSBInterfaceInterface800), 0, "Interface800"},
    {id_kIOUSBInterfaceInterfaceID942, sizeof(IOUSBInterfaceInterface942), 0, "Interface942"},
};

#define PATCH(T, field, fn)                                                    \
    if (offsetof(T, field) + sizeof(void *) <= size) ((T *)copy)->field = (void *)fn

static void wrap_object(void **obj, const VtKind *k) {
    if (!obj || !*obj) return;
    pthread_mutex_lock(&g_lock);
    for (int i = 0; i < g_nvt; i++)
        if (g_vt[i].copy == *obj) { pthread_mutex_unlock(&g_lock); return; } // already wrapped
    if (g_nvt >= MAXOBJ) { pthread_mutex_unlock(&g_lock); return; }
    size_t size = k->size;
    void *copy = malloc(size);
    memcpy(copy, *obj, size);
    g_vt[g_nvt].copy = copy;
    g_vt[g_nvt].orig = *obj;
    g_nvt++;
    pthread_mutex_unlock(&g_lock);
    if (k->is_dev) {
        PATCH(DevVT, DeviceRequest, w_DeviceRequest);
        PATCH(DevVT, DeviceRequestAsync, w_DeviceRequestAsync);
        PATCH(DevVT, ResetDevice, w_ResetDevice);
        PATCH(DevVT, DeviceRequestTO, w_DeviceRequestTO);
        PATCH(DevVT, DeviceRequestAsyncTO, w_DeviceRequestAsyncTO);
        PATCH(DevVT, USBDeviceReEnumerate, w_USBDeviceReEnumerate);
    } else {
        PATCH(IfVT, ControlRequest, w_ControlRequest);
        PATCH(IfVT, ControlRequestAsync, w_ControlRequestAsync);
        PATCH(IfVT, ReadPipe, w_ReadPipe);
        PATCH(IfVT, ReadPipeAsync, w_ReadPipeAsync);
        PATCH(IfVT, WritePipe, w_WritePipe);
        PATCH(IfVT, WritePipeAsync, w_WritePipeAsync);
        PATCH(IfVT, ClearPipeStall, w_ClearPipeStall);
        PATCH(IfVT, AbortPipe, w_AbortPipe);
        PATCH(IfVT, ResetPipe, w_ResetPipe);
        PATCH(IfVT, ControlRequestTO, w_ControlRequestTO);
        PATCH(IfVT, ControlRequestAsyncTO, w_ControlRequestAsyncTO);
        PATCH(IfVT, ReadPipeTO, w_ReadPipeTO);
        PATCH(IfVT, WritePipeTO, w_WritePipeTO);
        PATCH(IfVT, ReadPipeAsyncTO, w_ReadPipeAsyncTO);
        PATCH(IfVT, WritePipeAsyncTO, w_WritePipeAsyncTO);
        PATCH(IfVT, ClearPipeStallBothEnds, w_ClearPipeStallBothEnds);
    }
    *obj = copy;
    logf_("wrapped %s object %p", k->name, (void *)obj);
}

// ---- plug-in QueryInterface hook ----------------------------------------------

static struct {
    void *copy;
    HRESULT (*qi)(void *, REFIID, LPVOID *);
} g_pi[MAXOBJ];
static int g_npi;

static HRESULT w_QueryInterface(void *self, REFIID iid, LPVOID *ppv) {
    void *vt = *(void **)self;
    HRESULT (*qi)(void *, REFIID, LPVOID *) = NULL;
    for (int i = 0; i < g_npi; i++)
        if (g_pi[i].copy == vt) qi = g_pi[i].qi;
    if (!qi) return E_NOINTERFACE;
    HRESULT hr = qi(self, iid, ppv);
    if (hr == S_OK && ppv && *ppv) {
        CFUUIDRef req = CFUUIDCreateFromUUIDBytes(NULL, iid);
        for (size_t i = 0; i < sizeof kinds / sizeof kinds[0]; i++)
            if (CFEqual(req, kinds[i].id())) {
                wrap_object((void **)*ppv, &kinds[i]);
                break;
            }
        CFRelease(req);
    }
    return hr;
}

static kern_return_t my_IOCreatePlugInInterfaceForService(io_service_t svc, CFUUIDRef pluginType, CFUUIDRef interfaceType,
                                                          IOCFPlugInInterface ***theInterface, SInt32 *theScore) {
    kern_return_t kr = IOCreatePlugInInterfaceForService(svc, pluginType, interfaceType, theInterface, theScore);
    if (kr == KERN_SUCCESS && theInterface && *theInterface && **theInterface &&
        (CFEqual(pluginType, kIOUSBDeviceUserClientTypeID) || CFEqual(pluginType, kIOUSBInterfaceUserClientTypeID))) {
        IOCFPlugInInterface **pi = *theInterface;
        pthread_mutex_lock(&g_lock);
        if (g_npi < MAXOBJ) {
            IOCFPlugInInterface *copy = malloc(sizeof *copy);
            memcpy(copy, *pi, sizeof *copy);
            g_pi[g_npi].copy = copy;
            g_pi[g_npi].qi = (*pi)->QueryInterface;
            g_npi++;
            copy->QueryInterface = (void *)w_QueryInterface;
            *pi = copy;
        }
        pthread_mutex_unlock(&g_lock);
        logf_("plug-in created (%s)", CFEqual(pluginType, kIOUSBDeviceUserClientTypeID) ? "device" : "interface");
    }
    return kr;
}

__attribute__((used)) static const struct {
    const void *replacement;
    const void *replacee;
} interposers[] __attribute__((section("__DATA,__interpose"))) = {
    {(const void *)my_IOCreatePlugInInterfaceForService, (const void *)IOCreatePlugInInterfaceForService},
};

__attribute__((constructor)) static void init(void) { logf_("usbtrace loaded"); }
