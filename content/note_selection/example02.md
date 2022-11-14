---
title: "Example 2: Z2Z (Sapling)"
weight: 32
---

{{% notice info %}}
Amounts are in kzats (1 kzats = 1000 zats = 0.01 mZEC)
{{% /notice %}}

Let's assume we choose (Transparent, Sapling, Orchard) in the Pool Usage Order
which means we'd rather use our Transparent notes first and keep our Orchard notes.

Suppose we have the following notes:

- T: 50
- S: 50
- O: 50

Let's consider the following order:

| order # | T   | S   | O  |
|---------|-----|-----|----|
| 1       |     | 10  |    |

The number is the quantity for a given address type. 

- Order 1 is a z-addr for 10

## Note Selection 

We begin with (50, 50, 50) in each pool respectively.

- Order 1: we use 10 from S-pool. Now we have (50, 40, 50)

## Fee ZIP 327

In the T-pool, we have
- 0 inputs,
- 0 outputs

In the S-pool, we have
- 0 input,
- 1 output for Order #1

In the O-pool, we have
- 0 inputs,
- 0 outputs

1. T-pool contributes 0 logical actions
2. S-pool contributes 1 logical actions = max(1, 1)
3. O-pool contributes 0 logical actions

The number of logical actions = 1 and the fee is 1 * 5 = 5

However, we haven't paid for the fee, and we haven't considered the change outputs yet.

## Paying for the fee and making change outputs

We pay for the fee using 5 from S-pool, following our pool usage
preferences. 

{{% notice note %}}
If we follow the pool usage preferences exactly, we should be using
T-inputs, but they are disabled by default.
{{% /notice %}}

We also have change outputs:
- S: we used 50, pay 10+5 and we should get 35 back,

This output adds up to the transaction and modify the fee.

1. T-pool contributes 0 logical actions
2. S-pool contributes 2 logical actions = max(1, 2)
3. O-pool contributes 0 logical actions
   
The number of logical actions = 2 and the fee is 2 * 5 = 10

Let's adjust for the new fee.
- S: we used 50, pay 10+10 and we should get 30 back,

## Final transaction

- Inputs:
  - S: 50
- Outputs:
  - Order 1: S/10
  - Change: S/30
- Fee: 10


