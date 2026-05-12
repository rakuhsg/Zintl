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

void* zintlappkit_create_wgpu_surface(const void* window, ZintlRect rect);
void zintlappkit_destroy_wgpu_surface(const void* surface);
void zintlappkit_wgpu_surface_set_rect(const void* surface, ZintlRect rect);
void zintlappkit_wgpu_surface_drawable_size(const void* surface, uint32_t* out_width, uint32_t* out_height);
void* zintlappkit_wgpu_surface_metal_layer(const void* surface);

#ifdef __cplusplus
}
#endif

#endif
