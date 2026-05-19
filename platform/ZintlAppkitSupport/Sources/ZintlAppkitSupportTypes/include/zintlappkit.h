#ifndef _RUNTIL_RUNTILAPPKIT_H_
#define _RUNTIL_RUNTILAPPKIT_H_

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

typedef void (*ZintlWindowCommandCallback)(const void* user_data, const char* command_id);
typedef void (*ZintlWindowCommandRelease)(const void* user_data);

typedef struct {
    void (*did_create)(const void* user_data);
    void (*will_close)(const void* user_data);
    void (*did_close)(const void* user_data);
} WindowCallback;

void* zintlappkit_create_window(const void* user_data, const WindowCallback* callback);
void zintlappkit_show_window(const void* window);
void zintlappkit_window_set_bounds(const void* window, ZintlRect bounds);
void zintlappkit_window_set_size(const void* window, double width, double height);
void zintlappkit_window_set_position(const void* window, double x, double y);
void zintlappkit_window_set_commands(
    const void* window,
    const char* commands_json,
    const void* user_data,
    ZintlWindowCommandCallback callback,
    ZintlWindowCommandRelease release
);
void zintlappkit_destroy_window(const void* window);

void* zintlappkit_create_wgpu_surface(const void* window, ZintlRect rect);
void zintlappkit_destroy_wgpu_surface(const void* surface);
void zintlappkit_wgpu_surface_set_rect(const void* surface, ZintlRect rect);
void zintlappkit_wgpu_surface_drawable_size(const void* surface, uint32_t* out_width, uint32_t* out_height);
void* zintlappkit_wgpu_surface_metal_layer(const void* surface);

#ifdef __cplusplus
}
#endif

#endif
