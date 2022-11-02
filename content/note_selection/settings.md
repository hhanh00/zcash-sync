---
title: Settings
weight: 20
---

To make a payment, we take funds from notes we received previously but haven't spent yet and
we send the funds to the receipients' addresses.

## Inputs

Our inputs are the UTXO (for transparent funds), Sapling notes and Orchard notes.
Notes from different types are not directly fungible, but we can make a transaction
that uses several types of notes. Also, notes must be used as a whole. We cannot
use a fraction of a note. However, we can make outputs that send the funds back
to the origin address. This mimics the behavior of classic bank notes and coins.

We use the term UTXO for any kind of note, whether transparent, Sapling or Orchard.

## Settings

The transaction builder has the following settings:

- Privacy Policy: determines which kind of information leaking is allowed, if any. SamePoolTypeOnly by default
- Use Transparent Source: if true, the transaction can use transparent funds. False by default
- Pool Usage Priority: See below. (Orchard, Sapling, Transparent) by default
- Change Address: the address where the change is sent. It can be a Unified Address
that has multiple receivers. Address of the sender's account by default

## Privacy Policy

From the strictest policy to the loosest policy
- SamePoolOnly. Funds cannot leave their pool: t2t, sapling to sapling, orchard to 
orchard
- SamePoolTypeOnly. Funds can travel from one shielded pool to the other
- AnyPool. We do what it takes to build the transaction

## Pool Usage Order

The pool usage order is the order in which we take funds from the source.
For example, if we have: 10 ZEC in each pool and we want to spend 15 ZEC,
- when the pool priority is TSO, we take first from Transparent, then Sapling,
then Orchard. Therefore: 10 is taken from Transparent and 5 from Sapling.
- when the pool priority is OST, we take first from Orchard, then Sapling,
then Transparent. Therefore: 10 is taken from Orchard and 5 from Sapling.

Note that the "Use Transparent Source" takes priority over "Pool Usage Priority".
If it's unchecked, then we will not use transparent funds at all.

## Fees and Change

Fees are calculated based on the current network rules:
- either as a constant 1000 zats,
- or as ZIP-317 (pending)

The change follows the same policy rules as other recipients.
Therefore, it may get split into several outputs.
