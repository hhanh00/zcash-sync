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

