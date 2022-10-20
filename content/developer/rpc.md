---
title: RPC
weight: 10
---

There are several ways to use a warp sync in your project.

The simplest way is to go through the RPC API. 
Install and use warp sync as a server or a microservice for synchronization
and account maintenance.  

In this case, you should use the REST API published on 
[SwaggerHub](https://app.swaggerhub.com/apis/HANHHUYNHHUU/warp-sync_api/1.2.15)
. It is also available here in the section [REST API]({{< relref "rest" >}}).

## Build

First of all, you have to compile the server with `cargo`.

- Make sure you have downloaded the ZKSNARK parameters. If you have a working
installation of `zcashd`, the parameters are already downloaded and available.
If not, use the script `fetch-params.sh`
- Then run the following command
```shell
  cargo b -r --bin warp-rpc --features=rpc 
```

It's typical 100% Rust project.

## Configuration

Then set a configuration file `Rocket.toml`.

- `allow_backup`: enables the API that shows the seed phrases and should not be
turned on for public servers,
- `allow_send`: enables the API that builds and signs transactions, and should 
also be restricted for public servers.

A typical configuration file looks like:
```toml
[default]
allow_backup = false
allow_send = false

yec = { db_path = "./yec.db", lwd_url = "https://lite.ycash.xyz:9067" }
zec = { db_path = "./zec.db", lwd_url = "https://mainnet.lightwalletd.com:9067" }
```

{{% notice note %}}
WarpSync supports multiple coins and multiple accounts, therefore you need to set an active account before
calling the account methods.
{{% /notice %}}

another way to use a websync is to you have to use the RPC so use the RPC to initiate what the synchronization and use the RPC to do payments and getting your balance also you can still use the RPC as before but then you can also access the database directly because uh the database has a is a uh meaty process and as long as you use the the proper client library and implements that has locking you should be able to uh to connect to the database in the second secondary connection and directly use the database so if you do that you have to to be mindful that only only queries are allowed should be should be should be used if you modify that the database in any way it could be you could create a problems so it's not meant to be a to be a read write in this case you should consider database as read only but you have access to well notes accounts and uh history or this software and you can follow this keyboard that is below to uh to understand how these tables link together it's a simple relational database with only a few a few usable tables useful tables and another way to use WAP sync is uh if you also so these two ways to use option the final way to use swapsing is what is what uh let's see what it does which is to use it through the uh C C ffi file functional interface in this case what you would need is to compile a warp sync with the top ffi feature option which uh well the name is not really indicative exactly what he does is actually not as well it's not 100 specific to that because it creates a CEO a C library that you can use from other languages that supports C bindings uh dot does so that's why it's called Dot ffi with the c bandings you can you can call from per python all these guys and uh the okay so the way to use that is uh to look take a look at uh take a look at what here at DPI and the inventory point is dot effect so you will see that in Dot ffi

these are one of the uh expose for sure to see and this function is called set active account and the information implementation will delegate to the real to the real rust code so these functions here are only for wrapping and unwrapping unwrapping the parameters and wrapping the results and they translate pretty simply to uh to the C parameters so coin is a is a chart u8 is a Char and Adobe URL is a C string you can see that here if the function can return an error so these functions here are either do not return anything or they return a string but if they can return an error but we return is a c result which uh which is a parameterized type for for the return the real return value and the C result is similar to our past result except that it's a c structure that can be passed across a c boundaries uh so it's not Union it's always the value and error except that the error is a is a C string so if there's are no errors if the return is correct we have a value then error would be a normal pointer and if you have an error then error would be a C string again the C string pointing to to the r message and uh it's the uh so whenever you have a string that is returned to this to the C return to C uh it has been allocated by rust obviously but then you have to de-allocate it so there's a function a utility function for the allocating strings that we're allocating in in the rust uh it's called this one the allocate Str and you're supposed to to pass the string that you got from rust and once you're done with it and it will uh it would be allocate so you have a bunch of these functions I'm gonna they are going to be uh oh yeah documented in rostock but the uh what they do is very similar to um to the RPC particularly RPC uh also goes through these uh this interface uh so if you don't have to if you can use it at microservice I recommend the micro service option the RPC option because uh any you have to deal with uh with uh linking Library C libraries you know how to deal with ffi and all this stuff that is introduces I would say a necessary unnecessary complexity in in some cases in many cases and just using uh Json up API is probably enough for most of the cases and finally if you want to have there's also what is it okay so no DPI I think somewhere

well maybe it's not exposed well so let's say that these are the only all the options for using a website and then you can use examples all right that's it