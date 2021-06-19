## Abstract

The initial synchronization of a wallet with the Zcash blockchain is one of the most often mentioned issues with light clients.

In some cases, users have to wait for several minutes or even hours before the application is entirely up to date.

It is a strong deterrent for many users.

We introduce a new method of synchronization that leverages parallel execution and several properties of the note commitment tree. The result is an initial synchronization time of a few seconds.

Our synchronization method does not incrementally update the wallet state block per block. Instead, it directly computes the state at the current block height. Therefore it reduces the amount of calculation immensely.




