---
title: FFI
weight: 40
---

You can build the Warp Sync library as a dynamic library and 
use it from your code as long as it supports interfacing
with native code.

Most programming languages have the ability to call into C
code. Therefore, you should be able to use FFI (Foreign Function Interface).

In this section, we'll describe the low-level C API.

## Build

- First edit the file `Cargo.toml` and change the library type
from `rlib` to `cdylib`
- Then compile:
```shell
cargo b -r --features=dart_ffi
```

This should create a dynamic library in the `target/release` directory.
On Linux, the file is named `libwarp_api_ffi.so`. Other platforms have
slightly different names.

Even if your programming language is not DART, the feature is `dart_ffi`
for historical reasons.

## Header file

The C header file is `binding.h`

