---
title: Decrypt
weight: 20
---

One of the major tasks in terms of computation is trial decryption.

Every output note (EPK, CIPHERTEXT, CMU) needs to be trial decrypted 
with every account Incoming Viewing Key (IVK).

There are currently more than 20 million notes (and it increases). 
If you have 2 accounts, it means having to trial decrypt 40 million times.

Typically, one trial decryption takes ~1 ms (the exact time depends on your CPU). 
Therefore trial decryption can make up for a substantial portion of the synchronization.

The decryption is performed by the note-encryption crate of `librustzcash`.

Optionally, on some platform WarpSync can use an alternate hardware accelerated
implementation.

The CIPHERTEXT in the Compact Output *only* contains the first 52 bytes
of the complete CIPHERTEXT.

{{% notice info %}}
Compact Outputs exclude the COUT and MEMO parts, therefore it is not possible
to decrypt the memo text or the *outgoing* notes.
{{% /notice %}}

However, we can identity outgoing transactions by the fact that they
use one of our UTXO.

## Trial Decryption Algorithm

For reference, here is an *overview* of the algorithm used for decrypting
a compact output using an IVK.

- The $E$ = EPK, Ephemeral Public Key is a point on the Jubjub curve serialized
in compressed form.
- The Diffie-Hellman shared secret is $ S = E^{\text{IVK}}$. Note that the
base point $E$ differs with every note and therefore we cannot optimize
the exponentiation by precomputing tables of powers of $E$.
- The decryption key $K$ is the Blake2b hash of $S$.
- The ciphertext is encrypted with Chacha20 using $K$.

Once again, the CIPHERTEXT in the Compact Output *only* contains the first 52 bytes
of the complete CIPHERTEXT.

It doesn't have the memo or the authentication digest. We cannot directly
check that the decrypted text is valid. However, the format of a note
restricts the byte values at some offsets. For example, the first byte
is the note version number. It has to be 2 since zip 212 activation. 

Furthermore, if by chance the plain note passed all format validation checks
and was still invalid, it would be caught by the CMU check.

We can compute the plain note commitment hash (CMU) and make sure it matches
the CMU from the Compact Output.

## Batch optimization

1. Getting the affine coordinates (u, v) of EPK involves computing the inverse of a 
field element. We can batch multiple inversions together and only have to 
calculate one inverse, at the expense of having to do more multiplications.
However, inversions are much more expensive than multiplications so it is still a net
gain.
3. The point exponentiation is carried out in Extended Coordinates because
then we don't have to do field inversions, and it's faster, but we 
have to normalize the result and return to Affine Coordinates. This conversion
uses a field inversion per point. Here again, we can batch these normalizations
and use a single inversion for all the notes of a batch.

## Multi Threading

If your CPU has several cores, trial decryption is automatically
performed in parallel. We use [Rayon](https://docs.rs/rayon/1.5.3/rayon/index.html).

Rayon is lightweight and convenient for introducing parallelism into existing code. 
It guarantees data-race free executions and takes advantage of parallelism when sensible, 
based on work-load at runtime.
