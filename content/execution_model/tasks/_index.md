---
title: Tasks
weight: 10
icon: task
---

Synchronization is made of three main tasks:

- Block Download / Spam filtering
- Trial Decrypt
- Spend Detection
- Update witnesses
- Retrieve Transaction Details

They are executed asynchroniously using the [Tokio](https://tokio.rs/) 
runtime.
