#ifndef ZPD_WINUI3_H
#define ZPD_WINUI3_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct zpd_winui3_app_context zpd_winui3_app_context;
typedef struct zpd_winui3_dispatcher zpd_winui3_dispatcher;
typedef struct zpd_winui3_dispatcher_source zpd_winui3_dispatcher_source;
typedef struct zpd_winui3_dispatcher_signaler zpd_winui3_dispatcher_signaler;
typedef struct zpd_winui3_window zpd_winui3_window;
typedef struct zpd_winui3_element zpd_winui3_element;

typedef struct zpd_winui3_string {
  const uint8_t* data;
  size_t length;
} zpd_winui3_string;

typedef struct zpd_winui3_thickness {
  double left;
  double top;
  double right;
  double bottom;
} zpd_winui3_thickness;

typedef struct zpd_winui3_grid_length {
  int32_t kind;
  double value;
} zpd_winui3_grid_length;

enum zpd_winui3_menu_item_kind {
  ZPD_WINUI3_MENU_ITEM = 0,
  ZPD_WINUI3_MENU_SUBMENU = 1,
  ZPD_WINUI3_MENU_SEPARATOR = 2,
};

typedef struct zpd_winui3_menu_item {
  int32_t kind;
  zpd_winui3_string id;
  zpd_winui3_string title;
  zpd_winui3_string key;
  uint32_t modifiers;
  bool enabled;
  const struct zpd_winui3_menu_item* children;
  size_t children_length;
} zpd_winui3_menu_item;

typedef struct zpd_winui3_menu {
  zpd_winui3_string title;
  const zpd_winui3_menu_item* items;
  size_t items_length;
} zpd_winui3_menu;

typedef void (*zpd_winui3_launch_fn)(const zpd_winui3_app_context*, const void*);
typedef void (*zpd_winui3_invoke_fn)(const void*);
typedef void (*zpd_winui3_string_fn)(const void*, zpd_winui3_string);
typedef void (*zpd_winui3_release_fn)(const void*);

int32_t zpd_winui3_application_run(
    const void* user_data,
    zpd_winui3_launch_fn launch,
    zpd_winui3_release_fn release);

zpd_winui3_dispatcher* zpd_winui3_app_dispatcher(const zpd_winui3_app_context* context);
int32_t zpd_winui3_application_exit(void);
zpd_winui3_window* zpd_winui3_window_create(void);

zpd_winui3_dispatcher* zpd_winui3_dispatcher_clone(const zpd_winui3_dispatcher* dispatcher);
void zpd_winui3_dispatcher_release(zpd_winui3_dispatcher* dispatcher);
bool zpd_winui3_dispatcher_try_enqueue(
    const zpd_winui3_dispatcher* dispatcher,
    int32_t priority,
    const void* user_data,
    zpd_winui3_invoke_fn invoke,
    zpd_winui3_release_fn release);
zpd_winui3_dispatcher_source* zpd_winui3_dispatcher_source_create(
    const zpd_winui3_dispatcher* dispatcher,
    const void* user_data,
    zpd_winui3_invoke_fn invoke,
    zpd_winui3_release_fn release);
void zpd_winui3_dispatcher_source_release(zpd_winui3_dispatcher_source* source);
zpd_winui3_dispatcher_signaler* zpd_winui3_dispatcher_source_signaler(
    const zpd_winui3_dispatcher_source* source);
zpd_winui3_dispatcher_signaler* zpd_winui3_dispatcher_signaler_clone(
    const zpd_winui3_dispatcher_signaler* signaler);
void zpd_winui3_dispatcher_signaler_release(zpd_winui3_dispatcher_signaler* signaler);
bool zpd_winui3_dispatcher_signaler_signal(
    const zpd_winui3_dispatcher_signaler* signaler);

void zpd_winui3_window_release(zpd_winui3_window* window);
int32_t zpd_winui3_window_set_title(zpd_winui3_window* window, zpd_winui3_string title);
int32_t zpd_winui3_window_resize(zpd_winui3_window* window, int32_t width, int32_t height);
int32_t zpd_winui3_window_activate(zpd_winui3_window* window);
int32_t zpd_winui3_window_close(zpd_winui3_window* window);
int32_t zpd_winui3_window_set_content(zpd_winui3_window* window, const zpd_winui3_element* element);
int32_t zpd_winui3_window_extend_content_into_title_bar(zpd_winui3_window* window, bool enabled);
int32_t zpd_winui3_window_set_title_bar(
    zpd_winui3_window* window,
    const zpd_winui3_element* element);
int32_t zpd_winui3_window_set_backdrop(zpd_winui3_window* window, int32_t backdrop);
int32_t zpd_winui3_window_set_menu_bar(
    zpd_winui3_window* window,
    const zpd_winui3_menu* menus,
    size_t menus_length,
    const void* user_data,
    zpd_winui3_string_fn invoke,
    zpd_winui3_release_fn release);
int32_t zpd_winui3_window_clear_menu_bar(zpd_winui3_window* window);

zpd_winui3_element* zpd_winui3_button_create(zpd_winui3_string title);
int32_t zpd_winui3_button_set_title(zpd_winui3_element* button, zpd_winui3_string title);
int32_t zpd_winui3_button_set_enabled(zpd_winui3_element* button, bool enabled);
int32_t zpd_winui3_button_set_click_handler(
    zpd_winui3_element* button,
    const void* user_data,
    zpd_winui3_invoke_fn invoke,
    zpd_winui3_release_fn release);
int32_t zpd_winui3_button_clear_click_handler(zpd_winui3_element* button);

zpd_winui3_element* zpd_winui3_text_create(zpd_winui3_string text);
int32_t zpd_winui3_text_set_text(zpd_winui3_element* text, zpd_winui3_string value);
int32_t zpd_winui3_text_set_wrapping(zpd_winui3_element* text, bool enabled);
int32_t zpd_winui3_text_set_alignment(zpd_winui3_element* text, int32_t alignment);

zpd_winui3_element* zpd_winui3_text_field_create(zpd_winui3_string value);
int32_t zpd_winui3_text_field_set_value(zpd_winui3_element* field, zpd_winui3_string value);
int32_t zpd_winui3_text_field_get_value(
    const zpd_winui3_element* field,
    const void* user_data,
    zpd_winui3_string_fn receive);
int32_t zpd_winui3_text_field_set_placeholder(zpd_winui3_element* field, zpd_winui3_string value);
int32_t zpd_winui3_text_field_set_read_only(zpd_winui3_element* field, bool read_only);
int32_t zpd_winui3_text_field_set_change_handler(
    zpd_winui3_element* field,
    const void* user_data,
    zpd_winui3_string_fn invoke,
    zpd_winui3_release_fn release);
int32_t zpd_winui3_text_field_clear_change_handler(zpd_winui3_element* field);

zpd_winui3_element* zpd_winui3_stack_panel_create(int32_t orientation, double spacing);
int32_t zpd_winui3_panel_append(zpd_winui3_element* panel, const zpd_winui3_element* child);
int32_t zpd_winui3_panel_clear(zpd_winui3_element* panel);

zpd_winui3_element* zpd_winui3_grid_create(void);
int32_t zpd_winui3_grid_set_rows(
    zpd_winui3_element* grid,
    const zpd_winui3_grid_length* rows,
    size_t length);
int32_t zpd_winui3_grid_set_columns(
    zpd_winui3_element* grid,
    const zpd_winui3_grid_length* columns,
    size_t length);
int32_t zpd_winui3_grid_add(
    zpd_winui3_element* grid,
    const zpd_winui3_element* child,
    int32_t row,
    int32_t column,
    int32_t row_span,
    int32_t column_span);

void zpd_winui3_element_release(zpd_winui3_element* element);
int32_t zpd_winui3_element_set_margin(zpd_winui3_element* element, zpd_winui3_thickness margin);
int32_t zpd_winui3_element_set_width(zpd_winui3_element* element, double width);
int32_t zpd_winui3_element_set_height(zpd_winui3_element* element, double height);
int32_t zpd_winui3_element_set_horizontal_alignment(zpd_winui3_element* element, int32_t alignment);
int32_t zpd_winui3_element_set_vertical_alignment(zpd_winui3_element* element, int32_t alignment);

#ifdef __cplusplus
}
#endif

#endif
