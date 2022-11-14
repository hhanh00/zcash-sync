---
title: Transaction Privacy
weight: 5
---

Zcash is arguably the most private cryptocurrency out there. Unlike
its rivals, it offers fully encrypted transactions that do not 
reveal any information about the source and destination of funds.

However, this doesn't mean that every transaction has the highest level
of privacy, because in Zcash, privacy is *optional*. 

Users can decide to use unencrypted transactions that are nearly identical
to Bitcoin transactions. In other words, Zcash transactions can be
public too.

Currently, a vast majority of the transactions are *not* using Zcash
privacy features.

In this article, we'll discuss how shielded wallets and Ywallet specifically
work to produce the most private transactions possible.

## Pools and Addresses

### Transparent

Zcash has several pools of funds. Essentially, it is as Zcash had several
different internal currencies that are inter exchangeable 1 to 1, but
with different privacy properties.

If you use a transparent address (they are easily recognizable by the leading
character: 't'), your coins are unencrypted and are in plain view, just as in
Bitcoin.

This applies to both sources of funds and destinations of a transaction.

For example, if you send coins to a t-addr, both the amount sent and
the t-addr are stored in clear in the Blockchain. Moreover, if the recipient
transfers these funds to another t-addr, both the origin and the destination
are public. Zcash calls these transactions "t2t" for transparent to transparent.

### Shielded

To benefit from Zcash privacy feature, one must use shielded coins and addresses.
Unfortunately, it is *not* as easy as transparent transactions.

First of all, you need a wallet that supports shielded ZEC such as YWallet 
(disclaimer: I am its author). Shielded wallets also need a much more 
complex process to synchronize the Blockchain. The main impact to the user is a
*longer* synchronization. 

If you are OK with all of that, I highly recommend using a shielded wallet.
Otherwise, you are missing out on the privacy of Zcash and all your 
transactions are public.

Secondly, there are 3 shielded pools!
- Sprout,
- Sapling and
- Orchard.

Sprout is mostly deprecated at this point and hardly supported anymore.
Sapling is currently the largest pool, but it is being replaced by Orchard.
Orchard was released less than 1 year ago. There are very few apps
that support the Orchard pool at this moment.

## Transactions

When you make a transaction, you are taking coins from your wallet
and sending them to one/many recipients. Your coins were either transparent
or shielded (Sapling or Orchard), and become transparent or shielded at
the destination address(es).

We saw earlier than
when a transaction takes coins from a transparent address and sends them
to a transparent address, the transaction is t2t. Similarly, if a transaction
takes coins from a shielded address and sends them
to a shielded address, the transaction is called z2z.

{{% notice note %}}
However, t2t, z2z, etc are oversimplifications since transactions can take both
transparent *and* shielded coins, and produce transparent *and* shielded coins.
{{% /notice %}}

## Transaction Parts

Without getting into details, let's describe the elements of
a transaction.

A transaction has:
- some metadata: version, etc,
- a transparent section
- a sapling section
- an orchard section

Each section has roughly the same structure. It has a set of
inputs that specify where the funds come from, and a set of 
outputs that specify where the funds go to.

In the transparent section, everything is in cleartext and
one can see addresses and amounts.

In the sapling and orchard sections, the information is encrypted
and can only be decrypted by someone who holds the viewing key,
i.e. the recipient. A special cryptographic data called a 
zero-knowledge proof ensures that the encrypted data is valid
without revealing its contents.

However, Sapling and Orchard sections reveal the net change
of funds that resulted from the transaction. The protocol
calls it `valueBalanceSapling` and `valueBalanceOrchard`.

`valueBalanceSapling` is the net value of Sapling spends minus
Sapling outputs. And `valueBalanceOrchard` is the same value
but for Orchard.

{{% notice info %}}
YWallet shows net changes as Outputs - Spends, i.e.
the opposite of value balance because a net value is 
usually the difference between the "after" value and 
the "before" value.
{{% /notice %}}

{{% notice note %}}
`valueBalanceSapling` and `valueBalanceOrchard` are commonly
referred to as the "turnstiles".
{{% /notice %}}

## Transaction Privacy

A highly private transaction reveals little to no information.
It must not contain any transparent inputs or outputs and
have a low value of `valueBalanceSapling` and `valueBalanceOrchard`.

In other words, try to:

{{% notice warning %}}
- avoid using transparent inputs and outputs,
- use shielded funds from the same pool.
{{% /notice %}}

That's easier said than done. Many factors impact your source
of funds and the recipients of your transactions. If your receive
funds from someone who uses a transparent-only wallet, they
cannot send them to your shielded address. Likewise, if you want to
pay them you will have to send funds to a transparent address.

However, your wallet app can help you manage your funds.

## Fund Management

Let's see how this works with a concrete example.

For instance, someone just sent you 100 ZEC through their
transparent-only wallet and you now have 100 ZEC in your
transparent address.

{{< img "01_taddr.png" >}}

You want to shield some of these funds so that they are no longer
visible on the blockchain.

{{< img "02_shield.png" >}}

Ywallet will opt to shield *equally* as Sapling and Orchard notes.

This is so that you can make highly private transactions with both
Sapling and Orchard recipients.

If it sent everything to its Orchard address, you would not be able
to pay a Sapling address without crossing the turnstile from Orchard
to Sapling.

Similarly, if it sent everything to its Sapling address, you would not be able
to pay an Orchard address without crossing the turnstile from Sapling
to Orchard.

The most versatile distribution is 50% Sapling and 50% Orchard.

{{< img "03_shield.png" >}}

This transaction has "VERY LOW PRIVACY" since it involves transparent inputs
and outputs.

{{% notice info %}}
**VERY LOW PRIVACY** transactions have transparent inputs and outputs.
**LOW PRIVACY** transactions have transparent outputs but no transparent inputs.
{{% /notice %}}

After the shielding transaction, we now have two notes: one Sapling (in white)
and one Orchard (in yellow). They are 45 ZEC each.

{{< img "04_notes.png" >}}

Now, if we make a payment to a Sapling address of 45 ZEC or less, the 
wallet can just use the Sapling note and therefore not reveal any amount
(except for the fees).

{{% notice info %}}
**HIGH PRIVACY** transactions do not have transparent elements and
just reveal the fee going through the turnstiles.
{{% /notice %}}

{{< img "05_z2z.png" >}}

But if we go higher than 45 ZEC, the wallet needs to use the Orchard note
as well, and this will make some Orchard funds cross to the Sapling pool.

{{< img "06_z2z.png" >}}

The revealed amount is the excess that could not be covered by our Sapling note.
We are not showing the exact amount of the transaction, but it is less private
than before.

{{% notice info %}}
**MEDIUM PRIVACY** transactions do not have transparent elements but
reveal a larger amount than the fee going through the turnstiles.
{{% /notice %}}

## UA to the Rescue

With Multi-Receiver UA, i.e addresses that include multiple address
types, the recipient shows their ability to get funds in several 
pools.

Let's say that we need to pay the same amount as before (50 ZEC)
but were given a UA that has Sapling and Orchard receivers.

Now Ywallet can automatically split the payment between Sapling and Orchard
and use both notes without going through the turnstiles.

{{< img "07_z2zo.png" >}}

Moreover, it was able to keep the sender's pools balanced since after the
transaction there is still the same amount in both shielded pools.

The sender's wallet keeps optimal shielded spending potential.

## Multi Payment

Even with more complex transactions that involve multiple recipients and
address types, Ywallet can still compute an optimal execution plan.

For instance, let's consider the case of a 6-recipient transaction that has:
1. a Sapling address
2. an Orchard UA
3. a Transparent & Sapling UA
4. a Transparent & Orchard UA
5. a Sapling & Orchard UA
6. and finally a Transparent, Sapling and Orchard UA.

All address types are included except the t-addr which would ruin our privacy.

{{< img "08_multi.png" >}}

Ywallet comes up with the following transaction:

{{< img "09_multi.png" >}}

When offered the option to use Transparent or Shielded, Ywallet always chooses
Shielded: Recipients #3 & #4 are fully shielded.

When the recipient has both Sapling and Orchard receivers (#5 and #6)
Ywallet computes amounts that sum up to the requested amount but also
maximizes your privacy and your future shielded spending potential.

The transaction is **HIGHLY private** and we spend an **equal amount** of Sapling and
Orchard notes for it.
