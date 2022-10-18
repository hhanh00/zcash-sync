---
title: Witnesses
weight: 30
---

## Note Commitments

When someone makes a transactions and therefore creates output notes, they
are encrypted. Even when they are spent, notes are not revealed because
that would also give the transaction that created them.

In fact, we could imagine a scheme where the notes are not stored
in the blockchain but only the zk proofs. Of course, it implies that
clients have another way to retrieve the encrypted output if they
don't store it themselves.

In any case, the point is that when a transaction output, or note
is created we cannot blindly trust that it does not double spend
or just has a value that exceeds the inputs.

In a public blockchain, full nodes can verify every transaction
and ensure that they are well-formed. But if the transaction is
encrypted, it is impossible to check them. 

Thanks for zero knowledge proofs, we have now the technology to
create cryptographic proofs that the outputs follow the protocol
rules without knowing precisely their value.

But the ZKP is not enough. When we want to spend one of our note,
we must also show that what we spend is indeed a previously 
unspent output.

The usual tool for that is the *cryptographic commitment*.

Whenever you want to show that you know something but you don't want
to reveal it *at the moment*, you can use a commitment.

A commitment scheme works in two phases.

- You "commit" to your value by calculating a "commitment value". You
*publish* this value for everyone to see.
- When you want to show that you have the value, you reveal it
and people can check that the commitment value matches the published commitment.

For it to work, the commitment value must be:

- binding: You must reveal the right value. If you reveal something else, the
commitment value will *not* match.
- hiding: The commitment value is public but it must not give any information
that could reveal the source value. 

{{% notice note %}}
In Zcash, the note commitments ensure that when the notes are
used, they refer to outputs of previous transactions.
{{% /notice %}}

Output notes are encrypted but their commitments are public.

## Note Commitment Tree

Note commitments are put in the Blockchain and the order
in which they appear defines the order in which they added to a binary
tree of height 63. This tree is called the Note Commitment Tree.
Every zcash node agrees on the same commitment tree since it only
contains public information.

This tree has millions of leaves. They are never removed or modified.
Therefore the tree keeps growing and is not prunable at the moment.

If you receive a note from a transaction, the note is yours and
you have the note that matches the commitment. However, only a
small fraction of all the note commitments are yours. The rest
belong to other users.

When you want to spend a note, you must prove that it was a previous
transaction output. The way you prove that is by showing that the note
commitment is in the tree.

Now, showing that your note is in the tree without telling which note it
is, is difficult.

It requires some cryptographic tools.

## Merkle Tree

## Pedersen Hash

## Merkle Path

## Spend Statement ZKP

