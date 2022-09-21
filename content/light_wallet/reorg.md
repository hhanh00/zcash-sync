---
title: Reorganization
weight: 50
---

Reorganizations happen when several miners submit different blocks
at the same height.

It is a rare occurence but it happens.

For example, if you have the following situation:

- Miner A submits block 1001 after block 1000
- Miner B submits a different block 1001
- Miner C submits a block 1002 based on miner B's block 1001

Now we have two chains that deviate at block 1000:

- one built from Miner A's block 1001
- the other from Miner B's block 1001 and Miner C's 1002

Since nodes cannot determine a-priori which chains will get longer after
seeing block 1001, some of them first follow Miner A while others
follow Miner B.

But when Miner C produces block 1002, Miner B's chain is now longer
than Miner A's.

{{%notice info%}}
Nodes have to follow the longest chain.
{{%/notice%}}

When nodes that were following Miner A's chain see the blocks from 
Miner B and C's chain, they *must* switch over.

This is called a block reorganization. Note that the nodes that were
already following Miner B and C do not switch. Therefore, a block
reorganization is a local event. 

## After effects

When a node has to perform a block reorganization, it must undo the
effects of the blocks that are no longer valid. In this case, Miner A's
block 1001. Every transaction from that block must be undone:

- notes that were spent are returned
- new notes that were created are destroyed

In other words, the node should return to the point before Miner A's 
block 1001 and process Miner B's block 1001 instead. Then it should
continue with Miner C's block 1002.

Failure to handle reorganization leads to *incorrect* state, so 
it's paramount that a wallet can undo a previously applied block.

However, in normal situation reorganizations are short lived.
It is very unlikely to have a reorganization longer than a few blocks.

We assume that reorganizations are always shorter than 100 blocks.
