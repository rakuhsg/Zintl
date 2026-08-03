#ifndef _ZINTL_ZINTLAPPKIT_H_
#define _ZINTL_ZINTLAPPKIT_H_

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    void (*on_launch)(const void*);
    void (*perform)(const void*);
    void (*will_terminate)(const void*);
} AppCallback;

typedef struct {
    double x;
    double y;
    double width;
    double height;
} ZintlRect;

typedef struct {
    const uint8_t* bytes;
    uintptr_t length;
} ZintlString;

typedef struct {
    ZintlString value;
    bool is_some;
} ZintlOptionalString;

typedef void (*ZintlCommandCallback)(
    const void* user_data,
    ZintlString command_id
);
typedef void (*ZintlCommandRelease)(const void* user_data);
typedef void (*ZintlWindowRelease)(const void* user_data);
typedef void (*ZintlControlAction)(const void* user_data);
typedef void (*ZintlControlRelease)(const void* user_data);
typedef void (*ZintlStringCallback)(void* user_data, ZintlString value);
typedef void (*ZintlSidebarSelectionCallback)(const void* user_data, ZintlString item_id);
typedef void (*ZintlSidebarRelease)(const void* user_data);

typedef struct {
    void (*did_create)(const void* user_data);
    void (*will_close)(const void* user_data);
    void (*did_close)(const void* user_data);
    void (*did_click)(const void* user_data);
    ZintlWindowRelease release;
} WindowCallback;

void* zintlappkit_create_window(
    const void* user_data,
    const WindowCallback* callback
);
void zintlappkit_show_window(const void* window);
void zintlappkit_window_set_bounds(const void* window, ZintlRect bounds);
void zintlappkit_window_set_size(const void* window, double width, double height);
void zintlappkit_window_set_position(const void* window, double x, double y);
void* zintlappkit_window_content_view(const void* window);
bool zintlappkit_window_set_sidebar(
    const void* window,
    ZintlString sidebar_json,
    const void* user_data,
    ZintlSidebarSelectionCallback callback,
    ZintlSidebarRelease release
);
void zintlappkit_window_clear_sidebar(const void* window);
void* zintlappkit_create_view(ZintlRect frame);
void zintlappkit_release_view(const void* view);
void zintlappkit_view_add_subview(const void* parent, const void* child);
void zintlappkit_view_remove_from_superview(const void* view);
void zintlappkit_view_set_frame(const void* view, ZintlRect frame);
void zintlappkit_view_set_translates_autoresizing_mask_into_constraints(
    const void* view,
    bool enabled
);
void* zintlappkit_create_button(ZintlString title);
void zintlappkit_button_set_title(const void* button, ZintlString title);
void zintlappkit_button_set_action(
    const void* button,
    const void* user_data,
    ZintlControlAction action,
    ZintlControlRelease release
);
void zintlappkit_button_clear_action(const void* button);
void* zintlappkit_create_text_field(ZintlString value, bool label);
void zintlappkit_text_field_set_string_value(const void* text_field, ZintlString value);
void zintlappkit_text_field_get_string_value(
    const void* text_field,
    void* user_data,
    ZintlStringCallback callback
);
void zintlappkit_text_field_set_placeholder_string(
    const void* text_field,
    ZintlOptionalString value
);
void zintlappkit_text_field_set_editable(const void* text_field, bool editable);
void zintlappkit_text_field_set_selectable(const void* text_field, bool selectable);
void* zintlappkit_layout_constraint_create(
    const void* first_view,
    int32_t first_attribute,
    int32_t relation,
    const void* second_view,
    int32_t second_attribute,
    double multiplier,
    double constant
);
void zintlappkit_layout_constraint_set_active(const void* constraint, bool active);
void zintlappkit_layout_constraint_set_priority(const void* constraint, float priority);
void zintlappkit_release_layout_constraint(const void* constraint);
void zintlappkit_set_commands(
    ZintlString commands_json,
    const void* user_data,
    ZintlCommandCallback callback,
    ZintlCommandRelease release
);
void zintlappkit_destroy_window(const void* window);

void* zintlappkit_create_wgpu_surface(const void* window, ZintlRect rect);
void zintlappkit_destroy_wgpu_surface(const void* surface);
void zintlappkit_wgpu_surface_set_rect(const void* surface, ZintlRect rect);
void zintlappkit_wgpu_surface_drawable_size(const void* surface, uint32_t* out_width, uint32_t* out_height);
void* zintlappkit_wgpu_surface_metal_layer(const void* surface);
void* zintlappkit_wgpu_surface_view(const void* surface);

#ifdef __cplusplus
}
#endif

#endif
