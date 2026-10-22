#ifndef FF13_SETTINGS_H
#define FF13_SETTINGS_H
#include <stdint.h>
#include <stddef.h>
#ifndef FF13_CDECL
#if defined(_WIN32)
#define FF13_CDECL __cdecl
#else
#define FF13_CDECL
#endif
#endif
/* v1 x86 descriptor contract. Strings are NUL-terminated UTF-8 ZTR keys;
 * pointer graphs, code and metadata must remain valid for process lifetime.
 * Every control/section has a stable ID, independent of labels and row numbers.
 * Bounds: 256 controls/provider, 64-byte IDs, 128-byte name, 192-byte labels,
 * 1024-byte help; optional labels/help can be empty. No callbacks during Query.
 */
#define FF13_SETTINGS_VERSION 1u
#define FF13_SETTING_CHECKBOX 1u
#define FF13_SETTING_INTEGER 2u
#define FF13_SETTING_FLOAT 3u
#define FF13_SETTING_BUTTON 4u
#define FF13_SETTING_COMBO 5u
#define FF13_SETTING_SECTION 11u
/* SECTION starts a new settings page/group until the next section.
 * Long sections overflow; empty sections are omitted. Controls before the first
 * section belong to General. Sections are optional; no hardcoded mod IDs. */
typedef struct FF13SettingsQueryContext {
    uint32_t size, version, game;
    const char *game_name, *backend_version;
} FF13SettingsQueryContext;
typedef struct FF13SettingExtension {
    uint32_t size, kind, version;
    const struct FF13SettingExtension *next;
} FF13SettingExtension;
/* Optional v1 control extensions. Unknown kinds are ignored; chains are bounded
 * to 16 records. Initialize header.size to sizeof(the complete extension),
 * header.version to 1 and header.next to NULL unless another extension follows.
 * State callbacks run when a control is displayed or used, outside host locks.
 * With no state extension a control defaults to VISIBLE | ENABLED. BUSY disables
 * input. State belongs to a control, not to its SECTION marker. */
#define FF13_SETTING_EXT_STATE 0x1001u
#define FF13_SETTING_EXT_PRESENTATION 0x1004u
#define FF13_SETTING_EXT_DISPLAY 0x1005u
#define FF13_SETTING_VISIBLE 1u
#define FF13_SETTING_ENABLED 2u
#define FF13_SETTING_BUSY 8u
#define FF13_SETTING_ADVANCED 1u
#define FF13_SETTING_RESTART_REQUIRED 2u
#define FF13_SETTING_EXPERIMENTAL 4u
typedef struct FF13SettingStateExtension {
    FF13SettingExtension header;
    uint32_t (FF13_CDECL *read_state)(void *user);
    void *user;
} FF13SettingStateExtension;
/* Presentation flags may also be attached to SECTION, applying notices to all
 * subsequent controls in that section. They do not hide or disable controls. */
typedef struct FF13SettingPresentationExtension {
    FF13SettingExtension header;
    uint32_t flags;
} FF13SettingPresentationExtension;
typedef struct FF13SettingDisplayExtension {
    FF13SettingExtension header;
    const char *prefix, *suffix; /* ZTR keys or UTF-8; at most 24 bytes each. */
    uint32_t decimals; /* 0..6, for numeric controls. */
} FF13SettingDisplayExtension;
typedef struct FF13SettingControl {
    uint32_t size, kind;
    const char *id, *label, *help;
    float min, max, step;
    float (FF13_CDECL *read)(void *user);
    void (FF13_CDECL *write)(void *user, float value);
    void *user;
    void (FF13_CDECL *activate)(void *user);
    uint32_t option_count;
    const char *const *options;
    const FF13SettingExtension *extensions;
} FF13SettingControl;
typedef struct FF13SettingsProvider {
    uint32_t size, version;
    const char *id, *name;
    uint32_t count;
    const FF13SettingControl *controls;
    uint32_t reserved_menu_count, reserved_menu_stride;
    const void *reserved_menus;
    uint32_t control_stride;
    const char *author, *release, *help, *website;
    float reserved_color[4];
    uint32_t reserved_flags;
} FF13SettingsProvider;
typedef const FF13SettingsProvider *(FF13_CDECL *FF13SettingsQuery)(const FF13SettingsQueryContext *);
/* Visitor receives registered ID (length-delimited), owning module and Query.
 * Called outside locks; return nonzero to stop enumeration with that status.
 * Providers appear only after successful init, and remain loaded for lifetime. */
typedef int32_t (FF13_CDECL *FF13SettingsVisitor)(const uint8_t *, size_t, size_t, FF13SettingsQuery, size_t);
typedef struct FF13SettingsApi {
    uint32_t version, size;
    /* init thread only; duplicate IDs owned by another module return CONFLICT. */
    int32_t (FF13_CDECL *register_provider)(const uint8_t *id, size_t length, FF13SettingsQuery query);
    int32_t (FF13_CDECL *visit_providers)(FF13SettingsVisitor visitor, size_t user);
} FF13SettingsApi;
#endif
