---
title: Example 10 - Multi Payments
weight: 40
---

## Summary

This example combines multiple orders of different address types.
It also demonstrates how the algorithm handles multiple receivers in
a unified address.

{{% notice info %}}
Amounts are in kzats (1 kzats = 1000 zats = 0.01 mZEC)
{{% /notice %}}

Let's consider the following orders:

| order # | T  | S  | O  |
|---------|----|----|----|
| 1       | 10 |    |    |
| 2       |    | 20 |    |
| 3       |    |    | 30 |
| 4       | 40 | 40 |    |
| 5       | 50 |    | 50 |
| 6       |    | 60 | 60 |
| 7       | 70 | 70 | 70 |

The number is the quantity for a given address type. 
If a row has more than one set of numbers,
it's the *total* amount.

- Order 1 is a t-addr for 10
- Order 4 is a ua with transparent and sapling receivers for 40
- Order 7 is a ua with transparent, sapling and orchard receivers for 70 in *total* across
T, S and O receivers

Suppose we have the following notes:

- T: 100
- S: 160
- O: 70 & 50

## Note Selection 
Let's assume we choose (Sapling, Orchard, Transparent) in the Pool Usage Order
and the change address is a UA with Transparent and Sapling receivers.

We begin with (100, 160, 120) in each pool respectively.

- Order 1: we use 10 from T-pool. Now we have (90, 160, [70, 50])
- Order 2: we use 20 from S-pool. Now we have (90, 140, [70, 50])
- Order 3: we use 30 from O-pool. Now we have (90, 140, [40, 50])
- Order 4: we use 30 from S-pool. S is preferred over T. 
Now we have (90, 100, [40, 50]). 
- Order 5: we use 50 from O-pool. O is preferred over T.
  Now we have (90, 100, 40).
- Order 6: we use 60 from O-pool. S is preferred over O.
  Now we have (90, 40, 40).
- Order 7: we use 40 from S-pool and 30 from the O-pool. 
S is preferred over O, but we don't have enough and we need the O-pool too.
  Now we have (90, 0, 10).

## Fee ZIP 327

In the T-pool, we have 
- 1 input,
- 1 output for Order #1

In the S-pool, we have
- 1 input,
- 4 outputs for Order #2, #4, #6, #7

In the O-pool, we have
- 2 inputs,
- 3 outputs for Order #3, #5, #7

1. T-pool contributes 1 logical actions = max(1, 1).
2. S-pool contributes 4 logical actions = max(1, 4)
3. O-pool contributes 3 logical actions = max(2, 3)

The number of logical actions = 8 and the fee is 8 * 5 = 40

However, we haven't paid for the fee, and we haven't considered the change outputs yet.

## Paying for the fee and making change outputs

We pay for the fee using 10 from O-pool and 30 from T-pool, following our pool usage
preferences.

So now, we should have (60, 0, 0). 
But we spent all our inputs therefore, we need a change output of 60 to our t-addr.

This T-output increases the number of T-logical actions to 2 = max(1, 2).
Then the total number of logical actions is now 9. The fee becomes 45.

The change is also adjusted to 55.

## Final transaction

- Inputs: all notes are used
  - T: 100
  - S: 160
  - O: 70 & 50
- Outputs: 
  - Order 1: T/10 
  - Order 2: S/20
  - Order 3: O/30
  - Order 4: S/40
  - Order 5: O/50
  - Order 6: S/60
  - Order 7: S/40, O/30
  - Change: T/55
- Fee: 45

## Test Code
```rust
fn test_example10() {
  env_logger::init();
  let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
  config.use_transparent = true;
  config.privacy_policy = PrivacyPolicy::AnyPool;
  config.precedence = [ Pool::Sapling, Pool::Orchard, Pool::Transparent ];

  let utxos = [utxo!(1, 100), sapling!(2, 160), orchard!(3, 70), orchard!(4, 50)];
  let mut orders = [t!(1, 10), s!(2, 20), o!(3, 30), ts!(4, 40), to!(5, 50), so!(6, 60), tso!(7, 70)];

  let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
  println!("{}", serde_json::to_string(&tx_plan).unwrap());
}
```