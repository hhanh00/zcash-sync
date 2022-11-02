---
title: "Example 6: T2Z (Sapling)"
weight: 35
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

| order # | T   | S   | O   |
|---------|-----|-----|-----|
| 1       |     | 10  |     |

The number is the quantity for a given address type. 

## Settings

{{% notice note %}}
By default, the transaction will not cross from shielded to transparent.
We need to set the privacy policy to `AnyPool`.

We should **enable the usage of the transparent pool** and
**disable the usage of the shielded pool**.
{{% /notice %}}

Otherwise, the transaction will be z2z as it is more private than a t2z.

We'll skip the detailed explanation from now on.

## Final transaction

- Inputs:
  - T: 50
- Outputs:
  - Order 1: S/10
  - Change: T/30
- Fee: 10



