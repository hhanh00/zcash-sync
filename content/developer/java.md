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

## Example using JAVA

Checkout the `integrations/java` directory for an example of 
how to use JAVA with Warp Sync to create a new account
and query the seed phrase and address.

### Main class

```java
package app.ywallet;

import java.sql.*;

/**
 */
public class App
{
    static {
        System.loadLibrary("java_warp");
    }

    public static void main( String[] args ) throws Exception
    {
        Class.forName("org.sqlite.JDBC");
        final App app = new App();

        // Create a new account
        final int id = app.newAccount();

        // Connect to the database via JDBC
        Connection conn = DriverManager.getConnection("jdbc:sqlite:zec.db");

        // Query the seed and address of the account by id
        String query = "SELECT seed, address FROM accounts WHERE id_account = ?";
        PreparedStatement statement = conn.prepareStatement(query);
        statement.setInt(1, id);
        ResultSet rs = statement.executeQuery();
        while (rs.next()) {
            String seed = rs.getString(1);
            String address = rs.getString(2);

            System.out.println("seed phrase: " + seed + ", address: " + address);
        }
    }

    private native int newAccount();
}
```

### JNI Wrapper

The JNI wrapper calls `new_account` and returns the new account id.
In a more realistic case, the wallet would be initialized only once
and the account name would be passed in.

```c++
JNIEXPORT jint JNICALL Java_app_ywallet_App_newAccount
  (JNIEnv *, jobject) {
    init_wallet((char *)".");
    CResult_u32 result = new_account(0, (char *)"test", (char*)"", 0);
    return result.value;
}
```

### Makefile

The Makefile builds the JNI library that should be copied into the JAVA
lib path.

```makefile
libjava_warp.so:

app_ywallet_App.o: app_ywallet_App.cpp
	g++ -c -fPIC -I${JAVA_HOME}/include -I${JAVA_HOME}/include/linux app_ywallet_App.cpp

libjava_warp.so: app_ywallet_App.o
	g++ -shared -fPIC -o libjava_warp.so app_ywallet_App.o  -L/usr/lib -lwarp_api_ffi
```
