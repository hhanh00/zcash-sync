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

We want to show that we have the note that matches a commitment that 
was transmitted earlier and is now part of the tree.

This will also explain why the note commitments are stored as a tree
and now as a plain list.

The note commitment tree is organized as a binary tree. For each non-terminal
node, i.e. internal node, there are two children. The tree has a fixed depth
of 63, for a total of 2^63 commitments. However, at this moment, there are 
about 20-30 million notes used. The vast majority of the leaves are unused.
Unused leaves have a commitment value of [0; 32] (32 bytes of 00).

The inner nodes are hashes of the two children. Since each child is a hash
value (the leaf is also a hash value), they all have a size of 32 bytes.

{{% notice note %}}
To get the value of an inner node, concatenate the hash value of the left node
and the right node to form a 64 byte sequence and hash it.
{{% /notice %}}

Here's a small general Merkle Tree:
{{% img "Merkle-Tree-FA.jpg" %}}

It has only 3 levels and therefore can store $2^3=8$ elements 

In the case of the Merkle Tree for the note commitments (NCT), the layer $T_x$
is omitted because the data are already hashes.

Ex: $H_{EF} = \text{Hash}(H_E, H_F)$, where the Hash function is defined
in the next section.

$$H_{ABCD} = \text{Hash}(H_{AB}, H_{CD})$$
$$H_{ABCDEFGH} = \text{Hash}(H_{ABCD}, H_{EFGH})$$

Also, the NCT stores notes incrementally: previous entries are never removed
or modified. We just assign an empty slot to a new commitment in the order
they appear in the blockchain.

We can see that the root of the tree, $H_{ABCDEFGH}$ depends on the value of 
every note commitment.

Every time a new note gets added, the root hash will change. And because all
the nodes are hash values, it is impossible to predict in advance the root
value.

Also, the NCT is tracked by every network participant and is part of consensus.
We can query the current root hash from `zcashd`.

If we were only interested in the security of the note commitments, we wouldn't
need to build a Merkle Tree. We could simply build a list from all the note
commitments and hash them at once, as a sequence of bytes. If any of the 
commitment is altered, the overall hash would also change.

Instead of having all these intermediate hashes, we would have a single hash value
that combines all the hashes together. The calculation would be much faster
because only one hash would have to be computed.

But a Merkle Tree has a major advantage if you want to prove that a value belongs
to the set of hashes.

If you had the hash of the list, to prove that a value is part of that list, you
must:
- give every value of the list,
- have the verifier calculate the hash
- check it equals the "official" root hash
- check that the value you provided is in the list

Obviously, Giving 20-30 million hash values is not practical. 

With a Merkle Tree, we can reduce that amount of data to only 63 hashes.

Let's say we want to prove that $H_E$ is part of the tree.

We start by giving $H_E$. In this scenario, the receiver does not have $H_x$
but only knows the root hash.

In addition to $H_E$, we also give the "Merkle Path", which is the list of 
the *sibling hashes* from the leaf to the root

In our case, that would be $H_F$, $H_{GH}$, $H_{ABCD}$. The direct path
is $H_E$, $H_{EF}$ and $H_{EFGH}$, but we want the siblings.

The receiver/verifier can then recompute the direct path using the Merkle Path:
- $H_{EF} = h(H_E, H_F)$,
- $H_{EFGH} = h(H_{EF}, H_{GH})$,
- $H_{ABCDEFGH} = h(H_{ABCD}, H_{EFGH})$

The last hash is the root. If it matches, $H_E$ belongs to the tree.

We cannot fool the verifier because every data we provide is a hash. 
A main property of the hash function is that it computationnally impossible
to find *different* values $(a, b)$ than $(H_E, H_F)$ such as $H_{EF} = h(a, b)$

Therefore we are forced to give the real hash values if we want to match the
root hash.

In Zcash, this Merkle Path is an input to the Spend Statement ZKP that
we cover briefly in the last section.

## Pedersen Hash

As you can, the NCT contains a large number of hashes and needs to be updated
every time a new note is added. The hash function used is a Pedersen Hash.

The Pedersen Hash function securely maps a sequence of bits into a point on 
an elliptical curve, (Jubjub for Sapling).

It is much more expensive to compute a Pedersen Hash than to compute a SHA
or a Blake hash, due to the finite field arithmetic involved.

But it can be implemented in a ZK circuit more efficiently than a classic hash
function.

## Witnesses

For every UTXO that your wallet has, the app needs to update the Merkle Path
when we receive new outputs. Even if none of the outputs are ours. With
the growth of transaction outputs, we have now several million notes that have
to be processed in every wallet. 

## Warp Sync Optimizations

The main goal of Warp Sync is to minimize Pedersen Hash calculations 
and especially avoid recalculating the same hash twice.

When you have several notes in your wallet, there is a good chance
that they have part of their Merkle Path in common. A non-optimized
implementation would treat each note independently and update the
witnesses by recomputing their Merkle Path sequentially.

Warp Sync rebuilds the Merkle Tree in parallel, and distributes
the hash values to the note. It ensures that the calculation are
spread out across all the CPU core and that the witness updates
are essentially data copies.

Another advantage of doing hash calculations on the NCT instead
of on the witnesses, is that WS can use a cryptographic optimization,
that reduces the number of field divisions.

## Spend Statement ZKP

In the case of Zcash, the Merkle Path is not published and stored in
the blockchain but serves as a secret input to the ZKP spend statements.
Essentially, we state that we know the Merkle Path for the note 
we spend but we don't reveal it.
