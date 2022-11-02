---
title: Example 8 - Iterative Search
weight: 38
---

## Summary

This example demonstrates the iterative process towards finding a suitable
set of notes. Fees are calculated based on the structure of the transaction,
but they also impact the transaction. Therefore, the algorithm needs
to try out several combinations.

{{% notice info %}}
Amounts are in kzats (1 kzats = 1000 zats = 0.01 mZEC)
{{% /notice %}}

Let's consider the following orders:

| order # | T  | S  | O  |
|---------|----|----|----|
| 1       | 10 |    |    |

The number is the quantity for a given address type. 
If a row has more than one set of numbers, it's the *total* amount.

- Order 1 is a t-addr for 10

Suppose we have the following notes:

- T: 5, 7
- S: 12
- O: 10

## Note Selection 
Let's assume we choose (Sapling, Orchard, Transparent) in the Pool Usage Order
and the change address is a UA with Transparent and Sapling receivers.

We begin with (12, 12, 10) in each pool respectively.

- Order 1: we use 10 from T-pool. Now we have (2, 12, 10)

## Fee ZIP 327

In the T-pool, we have
- 2 inputs,
- 1 output for Order #1

In the S-pool, we have
- 0 input,
- 0 outputs

In the O-pool, we have
- 0 inputs,
- 0 outputs

1. T-pool contributes 2 logical actions = max(2, 1).
2. S-pool contributes 0 logical actions
3. O-pool contributes 0 logical actions

The number of logical actions = 2 and the fee is 2 * 5 = 10

However, we haven't paid for the fee, and we haven't considered the change outputs yet.

## Paying for the fee and making change outputs

We pay for the fee using 10 from T&S-pool, following our pool usage
preferences.

We also have change outputs:
- T: we use 12 (= 5+7), pay 10 to order #1 and 2 for fees,
- S: we use 12, pay 8 in fees, and we should get 4 back.

These outputs add up to the transaction and modify the fee.

1. T-pool contributes 2 logical actions = max(2, 1).
2. S-pool contributes 1 logical actions = max(1, 1).
3. O-pool contributes 0 logical actions
   
The number of logical actions = 3 and the fee is 3 * 5 = 15

Let's adjust for the new fee.

But we can only pay 12 from the S-pool. We need to use the O-pool too now.

- T: pay 2 for fees. No T-change
- S: pay 12 for fees. No more S-change
- O: pay 1 for fees, get 9 in change

We have to add an orchard action!

1. T-pool contributes 2 logical actions = max(2, 2).
2. S-pool contributes 1 logical actions = max(1, 0).
3. O-pool contributes 1 logical actions

The number of logical actions = 4 and the fee is 4 * 5 = 20

We need to pay 5 more from the O-pool since the S-pool is empty.
- T: pay 2 for fees. No T-change
- S: pay 12 for fees. No S-change
- O: pay 6 for fees, get 4 in change

At last, we are not modifying the transaction structure anymore.

## Final transaction

- Inputs: all notes are used
  - T: 5, 7
  - S: 12
  - O: 10
- Outputs:
  - Order 1: T/10
  - Change: O/4
- Fee: 20



