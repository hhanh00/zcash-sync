#ifndef __APPLE__
typedef char int8_t;
typedef unsigned char uint8_t;
typedef short int uint16_t;
typedef long long int int64_t;
typedef long long int uint64_t;
typedef long long int uintptr_t;
typedef long int int32_t;
typedef long int uint32_t;
#ifndef __cplusplus
typedef char bool;
#endif
#endif
typedef void *DartPostCObjectFnType;

typedef struct CResult {
  char value;
  char *error;
} CResult;
