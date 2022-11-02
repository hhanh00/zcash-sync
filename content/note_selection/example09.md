---
title: Example 9 - Failure
weight: 39
---

## Summary
This is the same as the previous example but we disable transparent
inputs. The transaction **fails** because it cannot pay for the fees.

{{% notice info %}}
Amounts are in kzats (1 kzats = 1000 zats = 0.01 mZEC)
{{% /notice %}}

Let's consider the following orders:

| order # | T  | S  | O  |
|---------|----|----|----|
| 1       | 10 |    |    |

The number is the quantity for a given address type. 

- Order 1 is a t-addr for 10

Suppose we have the following notes:

- T: 0
- S: 12
- O: 10

## Note Selection 
Let's assume we choose (Sapling, Orchard, Transparent) in the Pool Usage Order.

We begin with (0, 12, 10) in each pool respectively.

- Order 1: we use 10 from S-pool. Now we have (0, 2, 10)

## Fee ZIP 327

In the T-pool, we have
- 0 input,
- 1 output for Order #1

In the S-pool, we have
- 1 input,
- 0 output

In the O-pool, we have
- 0 input,
- 0 output

1. T-pool contributes 1 logical actions = max(0, 1).
2. S-pool contributes 1 logical actions = max(1, 0).
3. O-pool contributes 0 logical actions

The number of logical actions = 2 and the fee is 2 * 5 = 10

However, we haven't paid for the fee, and we haven't considered the change outputs yet.

## Paying for the fee and making change outputs

We pay for the fee using 2 from S-pool and 8 from O-pool, following our pool usage
preferences.

We also have change outputs:
- S: we used 12, pay 10 to Order #1, pay 2 in fees, we get 0 change
- O: we used 10, pay 8 in fees, we get 2 in change

These outputs add up to the transaction and modify the fee.

1. T-pool contributes 1 logical actions = max(0, 1).
2. S-pool contributes 1 logical actions = max(1, 0).
3. O-pool contributes 1 logical actions
   
The number of logical actions = 3 and the fee is 3 * 5 = 15

Let's adjust for the new fee.

But we cannot pay the additional 5 in fees since we only have 2 left.

## Final transaction

**Transaction failed** because of unsufficient funds (cannot pay fees)

If we *add another orchard note worth 10*, we get the final transaction:
- Inputs: all notes are used
  - S: 12
  - O: 10, 10
- Outputs:
  - Order 1: T/10
  - Change: O/2
- Fee: 20

The analysis is left as an exercise.

Hint:
1. T-pool contributes 1 logical actions = max(0, 1).
2. S-pool contributes 1 logical actions = max(1, 0).
3. O-pool contributes 2 logical actions = max(2, 1)
