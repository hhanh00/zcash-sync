---
title: Rollback
weight: 30
---

Warp Sync detects a reorganization when it receives a block from the server
that has a previous hash different from the hash of the latest block it has.

This means the server has switched over to a different sequence of blocks.

As explained in the page about [Reorganization]({{<relref "reorg">}}),
the wallet must rollback to a common previous state but we don't exactly
know when we deviated. Besides, we cannot rollback any number of blocks
because we can apply blocks but we cannot undo a block.

However, we have checkpoints and we can revert to a previous one.

## Revert to Checkpoint

Every row from the blocks, transaction, received notes 
and witnesses table
have a height value that indicates when the data was obtained.
To rollback to the state *after* block H, we just have to delete
every row that has a height greater than H.

And to undo the spends that happened after H, we also need to 
reset the spent field of any notes if the spent height is greater than 
H.

{{%notice info%}}
Warp Sync automatically handles reorganizations.
{{%/notice %}}
