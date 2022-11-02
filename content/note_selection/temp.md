---
title: Note Selection
weight: 100
draft: true
---

## Settings
- Prefer shielded over transparent?
- Prefer orchard over sapling?
- Can be expressed as a pool precedence:
    - (o, s, t)
    - (s, o, t)
    - (t, o, s)
    - (t, s, o)
- Not every combination is allowed: (o, t, s) is not possible

## Prepare pools

- Notes that have a value lower than the incremental fee are skipped. They cannot cover their transaction
  cost
- Compute the size of each pool (t, s, o) based on the Notes available
    - t: transparent if enabled. By default, using transparent funds is disallowed
    - s: sapling
    - o: orchard

## Prepare orders

- Create an order for each recipient
- Orders have a type based on the address:
    - t: t
    - z: s
    - ua: t+s, t+o, t+s+o, o, s+o
- and a value

## Overview

The fill algorithm links every order with every pool, but stops
if the order is completely filled.
When an order gets linked with a pool, it gets filled as much as possible
and it remains in the order book only if the pool is too small to completely
fill it.

From now on:

- If an order could be fully filled, remove it from the order book
- If the order book is empty, exit and go to "note selection"
- If the pool is empty, (t = 0, s = 0, o = 0) also exit and go to "note selection"

## Direct fills

Fill with identical source and destination pools

- (t) <- t
- (s) <- s
- (o) <- o
- (t+s) <- t, s: Use direct pool precedence
- (t+o) <- t, o: Use direct pool precedence
- (s+o) <- s, o: Use direct pool precedence
- (t+s+o) <- t, s, o: Use direct pool precedence

{{% notice info %}}
Use direct pool precedence: Use any pool that directly matches the destination.
If several matches exist, use the first one in the order of precedence.
{{% /notice %}}

## Cross shielded pool fills

Fill using only shielded pools

- (t) <- skip. shielded pools are not allowed
- (s) <- o
- (o) <- s
- (t+s) <- o
- (t+o) <- s
- (s+o) <- skip. all shielded pools were used
- (t+s+o) <- skip. all pools were used

{{% notice info %}}
Use indirect pool precedence: Use any pool that *does not* directly match the destination.
If several matches exist, use the first one in the order of precedence
selected by the user.
{{% /notice %}}

## Cross pool fills

Fill using t2z and z2t

- (t) <- s, o: Use pool precedence
- (s) <- t
- (o) <- t
- (t+s) <- skip. all pools were used
- (t+o) <- skip. all pools were used
- (s+o) <- t
- (t+s+o) <- skip. all pools were used

## Note Selection

- The result is an amount of the T, S, O pools that we need to find as inputs
- If any order has outstanding value, report "Insufficient Funds" error
- Save this pool allocation as $P$. It does not include a mining fee therefore is invalid.

We have the amount to spend in each pool, now we need to select the actual notes.
- Sequentially select notes from each pool until the total exceeds the required amount

## Fee

- For any $S_i$,
    - calculate the fee based on the number of inputs, outputs and actions
    - calculate the change $C$ as $\sum \text{inputs} - \sum \text{outputs} - \text{fee} $
- If $C$ is positive, the note selection is complete and we can exit
- Reset the pool allocation to $P$
- Create an order for $\text{fee}$ of type t+s+o
- Run the fee order through the fill algorithm
- It returns an amount of T, S, O like before
- Add it to $P$
- Run the note selection algorithm
- The result is $S_{i+1}$
- Go to the first step and check $S_{i+1}$

The algorithm is such that $S_{i+1} \supset S_i$ and therefore is guaranteed to terminate.