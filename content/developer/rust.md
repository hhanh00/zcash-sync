---
title: Java
weight: 50
---

## Example using Java

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
