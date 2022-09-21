---
title: Download
weight: 10
---

This task consists of retrieving Compact Blocks from the server and
sending them downstream to the other tasks.

Lightwalletd offers a streaming interface that allows us to retrieve
a range of blocks without blocking.

The start height is our latest synchronization height + 1.
The end height is the latest block. The range can potentially be
very large, but we don't buffer more than a chunk of blocks
and therefore can process large ranges.

## Block Chunks

Warp Sync does not process blocks individually but as a sub ranges
of the whole synchronization range.

When we receive a block, we filter out the transactions that have
too many outputs/actions (Spam Filter) by clearing their
output EPK and CIPHERTEXT.

We need to keep the CMU. They are needed for the Witness update.

The library queries the device available memory and adjusts the
maximum number of outputs per chunk with a cap of 200 000 outputs.

{{% notice note %}}
The outputs are counted before the spam filter. 
{{% /notice %}}

Otherwise, we could have an extremly high number of outputs to 
process in the "Update Witness" stage.

Once a chunk is full, we send it down through the 
[pipeline]({{<relref "pipeline">}}) to the
next processing stage.

{{% notice info %}}
Download continues asynchronously in parallel with block chunk processing.
{{% /notice %}}

