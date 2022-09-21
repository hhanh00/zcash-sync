---
title: Purpose
weight: 10
---

The main purpose of a Light Wallet or
any wallet for that matter is to provide:
- the capability to receive funds,
- the capability to send funds,
- and display the funds available

In addition, Light Wallet should be less resource-demanding than regular wallets. In the past, the latter term has been used to refer to full nodes.

Full node wallets download the entire blockchain and keep a fairly extensive representation of the data within.

As of the time of writing (Sep 2022), the Zcash blockchain takes ~150 GB of storage space. It includes the Blockchain, and the database used to store the notes and transactions.

At the same time, a light wallet for Zcash  "only" downloads 5 GB of data and keeps a few MB of storage.

This represents a gain of ~100X. They fit into mobile devices.
They are particularly useful to the casual user.

{{% notice note %}} 
The original bitcoin paper by Satoshi mentions the idea of light wallets.
He refers to them as Simplified Payment Verification wallets (SPV).
{{% /notice %}}

However, light wallets have also some disadvantages.

The fact that they do not download the full blockchain prevents them from validating its content and, consequently, 
from participating actively in the network. They must connect to light wallet servers that have processed the blockchain on their behalf. 
The wallets, however, need not fully trust the servers since another form of cryptographic validation is employed. 
A detailed description of the threat model for wallet apps can be found here: 
[Wallet App Threat Model](https://zcash.readthedocs.io/en/latest/rtd_pages/wallet_threat_model.html).

