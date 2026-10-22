/*
 * Crux host <-> extension-unit ABI.
 *
 * An extension unit is an ordinary 32-bit Windows DLL that exports
 *   int32_t __cdecl init(const FF13HostContext *ctx); // 0 = success
 *   void __cdecl shutdown(void);                    // optional
 * The host loads the unit, calls init() once (early, while the mod image loads, or
 * late, at field idle) and keeps it resident for the process lifetime.
 *
 * ABI stability: fields are only appended. A unit must check `version` and
 * use `size` to tell which trailing fields exist. The layout below is the
 * 32-bit (x86) layout used by Crux; the SDK's `abi::HostContext`
 * mirrors it.
 */
#ifndef FF13_HOST_H
#define FF13_HOST_H
#include <stddef.h>
#include <stdint.h>
#if defined(_WIN32)
#define FF13_CDECL __cdecl
#else
#define FF13_CDECL
#endif
#include "ff13_settings.h"
#include "ff13_save_events.h"
#ifdef __cplusplus
extern "C" {
#endif

#define FF13_HOST_ABI_VERSION 1u

/* HostContext::phase */
#define FF13_PHASE_EARLY 0u
#define FF13_PHASE_LATE  1u

/* Service status codes (negative are errors). */
#define FF13_OK               0
#define FF13_INVALID_ARGUMENT (-1)
#define FF13_NOT_FOUND        (-2)
#define FF13_SHORT_BUFFER     (-3) /* *written receives the required size */
#define FF13_DECODE_FAILED    (-4)
#define FF13_UNAVAILABLE      (-5)
#define FF13_HOOK_FAILED      (-6)

/* Log levels for FF13HostContext.log */
#define FF13_LOG_ERROR 0u
#define FF13_LOG_WARN  1u
#define FF13_LOG_INFO  2u
#define FF13_LOG_DEBUG 3u

/* The context and callbacks remain valid for the process lifetime after init.
 * Pointer arguments must address valid buffers of the stated length; output
 * buffers and size/written must be writable and must not overlap input data.
 * Paths are nonempty byte strings relative to the game data root, without a
 * trailing NUL. Callbacks do not retain caller buffers.
 */
/* message is UTF-8, not NUL-terminated; at most 4096 bytes are logged. */
typedef void (FF13_CDECL *FF13LogFn)(uint32_t level, const uint8_t *message, size_t length);
/* Decoded size of a served merged-container VFS path. */
typedef int32_t (FF13_CDECL *FF13VfsSizeFn)(const uint8_t *path, size_t length, uint64_t *size);
/* Whole decoded file; SHORT_BUFFER stores the needed size in *written.
 * output may be NULL only when capacity == 0. An empty file then returns OK.
 * Other errors leave *written unspecified; check the status before using it.
 */
typedef int32_t (FF13_CDECL *FF13VfsReadFn)(const uint8_t *path, size_t length,
                                 uint8_t *output, size_t capacity, size_t *written);
/* MinHook-backed helpers return FF13_* status codes, not MH_STATUS.
 * create is available only on the thread inside this unit's init call, so
 * failed initialization can clean up its hooks. It creates a disabled hook
 * and writes its trampoline to *original. enable/disable/remove may be used
 * later for extension-created targets. NULL/all-hooks and host-owned targets
 * are forbidden. Extensions must coordinate shared target ownership.
 */
typedef int32_t (FF13_CDECL *FF13HookCreateFn)(void *target, void *detour, void **original);
typedef int32_t (FF13_CDECL *FF13HookTargetFn)(void *target);

/* Optional host-owned persistent storage. One mod-state.dat in the mounted mod
 * root; little-endian packed bytes belong to the mod. Registration is init-only,
 * handles are session-local, IDs (1..64 ASCII letters/digits/._-) persist across
 * BIN updates. The on-disk key is the first 64 SHA-256 bits of the stable ID.
 * A second component claiming the same ID receives CONFLICT.
 * New regions return NOT_FOUND from info/read until their first write_batch.
 * All callbacks are thread-safe; never call from DllMain. A successful batch is
 * durable, includes resize/schema change, and replays atomically after a crash.
 * IO_ERROR means outcome may be committed; RECOVERY_REQUIRED blocks further use
 * until reopening on next launch. Do not retry with stale in-memory state.
 * Limits: 256 regions, 4 MiB per region, 256 ranges / 64 KiB per batch or read.
 * Decoded redo transaction (changed chunks/metadata) is capped at 128 KiB;
 * a widely scattered compressed batch can hit LIMIT with less than 64 KiB input.
 * Overlapping ranges are rejected. Growth zero-fills; shrink releases extents.
 * expected_revision=0 for a new region; info supplies subsequent revisions.
 * Compression values 0 and 1 are retained v1 client hints. Chunks below 128 bytes
 * stay raw; larger chunks use magicless zstd level 1 only when it saves >16 bytes.
 * Reads stream touched chunks with bounded scratch; regions are never preloaded.
 * Mode can change only while the old logical length is zero. Schema must be >0.
 * Newly registered/uninitialized regions return NOT_FOUND from info/read.
 * Successful writes are durable; IO_ERROR can have committed: reopen next session
 * before retrying. A poisoned service returns RECOVERY_REQUIRED thereafter.
 * Do not invoke storage while holding the Windows loader lock (e.g. DllMain).
 * Output buffers must not overlap any input; callbacks retain no caller buffers.
 */
#define FF13_STATE_VERSION 1u
#define FF13_STATE_IO_ERROR (-7)
#define FF13_STATE_CONFLICT (-8)
#define FF13_STATE_LIMIT (-9)
#define FF13_STATE_RECOVERY_REQUIRED (-10)
typedef struct FF13StateInfo {
    uint32_t size, schema;
    uint64_t length, revision;
    uint32_t compression, reserved;
} FF13StateInfo;
typedef struct FF13StateRange {
    uint64_t offset;
    const uint8_t *data;
    size_t length;
} FF13StateRange;
typedef struct FF13StateApi {
    uint32_t version, size;
    int32_t (FF13_CDECL *register_region)(const uint8_t *id, size_t length, uint32_t *handle);
    int32_t (FF13_CDECL *info)(uint32_t handle, FF13StateInfo *output);
    int32_t (FF13_CDECL *read_at)(uint32_t handle, uint64_t offset, uint8_t *output, size_t length);
    int32_t (FF13_CDECL *write_batch)(uint32_t handle, uint64_t expected_revision,
        uint64_t new_length, uint32_t schema, uint32_t compression,
        const FF13StateRange *ranges, size_t count, FF13StateInfo *output);
    int32_t (FF13_CDECL *flush)(void);
} FF13StateApi;

typedef struct FF13HostContext {
    uint32_t version;    /* = FF13_HOST_ABI_VERSION                       off 0  */
    uint32_t size;       /* sizeof(FF13HostContext) as built by the host  off 4  */
    uint32_t game;       /* 1 = XIII, 2 = XIII-2, 3 = Lightning Returns   off 8  */
    uint32_t phase;      /* FF13_PHASE_EARLY | FF13_PHASE_LATE            off 12 */
    size_t   game_base;  /* loaded main image base                       off 16 */
    size_t   game_size;  /* SizeOfImage                                  off 20 */
    FF13LogFn        log;          /*                                    off 24 */
    FF13VfsSizeFn    vfs_size;     /*                                    off 28 */
    FF13VfsReadFn    vfs_read;     /*                                    off 32 */
    FF13HookCreateFn hook_create;  /*                                    off 36 */
    FF13HookTargetFn hook_enable;  /*                                    off 40 */
    FF13HookTargetFn hook_disable; /*                                    off 44 */
    FF13HookTargetFn hook_remove;  /*                                    off 48 */
    const FF13StateApi *storage;   /* optional; check size >= 56          off 52 */
    const FF13SettingsApi *settings; /* optional; check size >= 60         off 56 */
    const FF13SaveEventsApi *save_events; /* optional, off 60; size >= 64 */
} FF13HostContext;                 /* sizeof == 64 on x86                        */

typedef int32_t (FF13_CDECL *FF13InitFn)(const FF13HostContext *ctx);
typedef void (FF13_CDECL *FF13ShutdownFn)(void);

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(offsetof(FF13HostContext, game_base) == 16, "ABI: game_base");
_Static_assert(offsetof(FF13HostContext, log) == 24, "ABI: first fn pointer");
_Static_assert(offsetof(FF13HostContext, hook_remove) == 48, "ABI: final fn pointer");
_Static_assert(sizeof(FF13HostContext) == 64, "ABI: context size");
#elif defined(__cplusplus) && __cplusplus >= 201103L
static_assert(offsetof(FF13HostContext, game_base) == 16, "ABI: game_base");
static_assert(offsetof(FF13HostContext, log) == 24, "ABI: first fn pointer");
static_assert(offsetof(FF13HostContext, hook_remove) == 48, "ABI: final fn pointer");
static_assert(sizeof(FF13HostContext) == 64, "ABI: context size");
#endif

#ifdef __cplusplus
}
#endif
#endif /* FF13_HOST_H */
