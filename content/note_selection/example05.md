---
title: "Example 5: Z2T (Orchard)"
weight: 35
---

{{% notice info %}}
Amounts are in kzats (1 kzats = 1000 zats = 0.01 mZEC)
{{% /notice %}}

Suppose we have the following notes:

- T: 50
- S: 50
- O: 50

Let's consider the following order:

| order # | T   | S   | O   |
|---------|-----|-----|-----|
| 1       | 10  |     |     |

The number is the quantity for a given address type. 

## Settings

{{% notice note %}}
By default, the transaction will not cross from shielded to transparent.
We need to set the privacy policy to `AnyPool`.

The pool usage should also **prefer Orchard over Sapling**.
For example, we can use (Transparent, Orchard, Sapling) as the Pool Usage Order.

{{% /notice %}}

## Note Selection 

We begin with (50, 50, 50) in each pool respectively.

- Order 1: we use 10 from S-pool. Now we have (50, 50, 40)

## Fee ZIP 327

In the T-pool, we have
- 0 inputs,
- 1 output for Order #1

In the S-pool, we have
- 0 input,
- 0 output

In the O-pool, we have
- 1 input,
- 0 output

1. T-pool contributes 1 logical actions
2. S-pool contributes 0 logical actions
3. O-pool contributes 1 logical actions

The number of logical actions = 2 and the fee is 2 * 5 = 10

However, we haven't paid for the fee, and we haven't considered the change outputs yet.

## Paying for the fee and making change outputs

We pay for the fee using 5 from O-pool, following our pool usage
preferences. 

We also have change outputs:
- O: we used 50, pay 10+5 and we should get 35 back,

This output adds up to the transaction and modify the fee.

1. T-pool contributes 1 logical actions
2. S-pool contributes 0 logical actions
3. O-pool contributes 1 logical actions = max(1, 1)
   
The number of logical actions = 2 and the fee is 2 * 5 = 10

Let's adjust for the new fee.
- O: we used 50, pay 10+10 and we should get 30 back,

## Final transaction

- Inputs:
  - O: 50
- Outputs:
  - Order 1: T/10
  - Change: O/30
- Fee: 10



