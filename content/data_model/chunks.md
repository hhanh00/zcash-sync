---
title: Block Chunks
weight: 20
---

Warp Sync processes groups of sequential blocks and not blocks one by one.

The size of the chunks are dynamically determined and varies based on the block
contents and device available resources.

At the end of a chunk, Warp Sync writes a checkpoint that allow a wallet
to resume processing if interrupted.

A checkpoint has the synchronization state at a given block height H:

- block height, hash, time, etc. stored in the blocks table,
- transactions that were made before and including H,
- received and spent notes before and including H.

{{%notice note%}}
Blocks between checkpoints are processed but not stored in the database.
{{%/notice %}}

{{%notice note%}}

{{%/notice %}}
