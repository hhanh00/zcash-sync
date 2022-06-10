# Build as standalone server

```
cargo b --release --bin warp-rpc --features=rpc
./target/release/warp-rpc
```

# Configuration

Edit `Rocket.toml`

```
[default]
allow_backup = true
allow_send = true
```

Edit `.env`

```
ZEC_DB_PATH
ZEC_LWD_URL
YEC_DB_PATH
YEC_LWD_URL
```

# RPC

TODO
```
set_lwd,
set_active,
new_account,
list_accounts,
sync,
rewind,
get_latest_height,
get_backup,
get_balance,
get_address,
get_tx_history,
pay,
```