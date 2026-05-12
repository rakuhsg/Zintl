#ifndef _RUNTIL_RUNTILAPPKIT_H_
#define _RUNTIL_RUNTILAPPKIT_H_

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    void (*on_launch)(const void*);
    void (*perform)(const void*);
    void (*will_terminate)(const void*);
} AppCallback;

#ifdef __cplusplus
}
#endif

#endif
