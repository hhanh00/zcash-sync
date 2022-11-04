# Testing on regtest

## Requirements

1. zcashd
2. zcash-cli
3. lightwalletd

Then,
* Both apps should be accessible from your `PATH`
* Create a directory and `cd` to it.

## Zcashd

- Configuration `zcash.conf`:
```toml
regtest=1
nuparams=c2d6d0b4:1
txindex=1
insightexplorer=1
experimentalfeatures=1
rpcuser=user
rpcpassword=s!NWfgM!5X55
```
- Start zcashd
- Check status
- Create a new account
- List addresses
- Mine 200 blocks
- Check balance
- Shield coinbase to our test zaddr
- Check result
- Mine 1 block

```sh
$ zcashd -datadir=$PWD --daemon
$ zcash-cli -datadir=$PWD getinfo
$ zcash-cli -datadir=$PWD z_getnewaccount
$ zcash-cli -datadir=$PWD listaddresses
$ zcash-cli -datadir=$PWD generate 200
$ zcash-cli -datadir=$PWD getbalance
$ zcash-cli -datadir=$PWD z_sendmany "ANY_TADDR" '[{"address": "zregtestsapling1qzy9wafd2axnenul6t6wav76dys6s8uatsq778mpmdvmx4k9myqxsd9m73aqdgc7gwnv53wga4j", "amount": 6.24999}]'
$ zcash-cli -datadir=$PWD z_getoperationresult
$ zcash-cli -datadir=$PWD generate 1
```

## Lightwalletd

- Start lightwalletd

```sh
$ lightwalletd --no-tls-very-insecure --zcash-conf-path $PWD/zcash.conf --data-dir . --log-file /dev/stdout
```

## Test zcash-sync

From project directory,

`Rocket.toml` should have

```toml
zec = { db_path = "./zec.db", lwd_url = "http://127.0.0.1:9067" }
```

- Build
- Run
- Create account using the test seed phrase: `bleak regret excuse hold divide novel rain clutch once used another visual forward small tumble artefact jewel bundle kid wolf universe focus weekend melt`
- Sync
- Check balance: 624999000

```sh
$ cargo b --features rpc --bin warp-rpc
$ ../../target/debug/warp-rpc 
$ curl -X POST -H 'Content-Type: application/json' -d '{"coin": 0, "name": "test", "key": "bleak regret excuse hold divide novel rain clutch once used another visual forward small tumble artefact jewel bundle kid wolf universe focus weekend melt"}' http://localhost:8000/new_account
$ curl -X POST 'http://localhost:8000/sync?offset=0'
$ curl -X GET http://localhost:8000/balance
```
