#ifndef FF13_SAVE_EVENTS_H
#define FF13_SAVE_EVENTS_H
#include <stddef.h>
#include <stdint.h>
#ifndef FF13_CDECL
#if defined(_WIN32)
#define FF13_CDECL __cdecl
#else
#define FF13_CDECL
#endif
#endif
#define FF13_SAVE_EVENTS_VERSION 1u
#define FF13_SAVE_BEGIN 1u
#define FF13_SAVED 2u
#define FF13_SAVE_FAILED 3u
#define FF13_LOADED 4u
#define FF13_NEW_GAME 5u
#define FF13_SAVE_EVENT_MASK(kind) (1u << ((kind) - 1u))
#define FF13_SAVE_ALL_EVENTS 31u
#define FF13_SAVE_SLOT_VALID 1u
#define FF13_SAVE_PROFILE_VALID 2u
#define FF13_SAVE_NO_SLOT UINT32_MAX
typedef struct FF13SaveEvent {
    uint32_t version, size, game, variant, kind, slot;
    uint64_t operation;
    uint32_t flags, reserved;
    uint8_t profile[32];
} FF13SaveEvent;
typedef struct FF13SaveEventInfo {
    uint32_t size, game, variant, events, slot_count, reserved;
} FF13SaveEventInfo;
/* Advisory result: nonzero is logged; it cannot veto or alter a retail save/load.
 * Called synchronously on the game's operation thread, without host registry or
 * storage locks. Host storage is safe to call. Never unwind, unload, or start a
 * nested game save/load. Copy the event if work is deferred; its pointer expires
 * when the callback returns. Keep callbacks short.
 * SAVE_BEGIN can snapshot state; SAVED/SAVE_FAILED carry its operation token.
 * LOADED means accepted game state, not a slot preview/list read. NEW_GAME has
 * NO_SLOT. Use flags before slot/profile; missing identity must never mean slot0.
 * Storage in mod-state.dat is independent of the retail save: no cross-file
 * atomicity is promised. Persist only after SAVED; reset transient state on NEW_GAME.
 */
typedef int32_t (FF13_CDECL *FF13SaveEventCallback)(const FF13SaveEvent *, uintptr_t user);
typedef struct FF13SaveEventsApi {
    uint32_t version, size;
    int32_t (FF13_CDECL *info)(FF13SaveEventInfo *output);
    /* Init-only; stable unique ID, nonzero event mask. Published after successful
     * init. Callback/user must remain valid for the module's process lifetime.
     * Hooks are installed on demand. Unsupported adapters return UNAVAILABLE;
     * mismatching/conflicting hooks return HOOK_FAILED. No guessed addresses.
     */
    int32_t (FF13_CDECL *register_callback)(const uint8_t *id, size_t length,
        uint32_t events, FF13SaveEventCallback callback, uintptr_t user);
} FF13SaveEventsApi;
#endif
