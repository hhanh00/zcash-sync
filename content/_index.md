---
title: "WarpSync"
archetype: home
weight: 10
---

Welcome to the documentation website for WarpSync.

WarpSync is a fast synchronization library for Zcash. 

## Overview

This documentation starts with an overview of the purpose 
and architecture of [Lightwallets]({{< relref "light_wallet" >}}). 
It describes the functionalities
required by a synchronization library.

The next section describes the [Data model]({{< relref "data_model" >}}). 
Warp Sync stores its
data in a SQLite Database. Each major table is shown and its
purpose explained.

Then, we show the synchronization workflow in the section
[Execution Model]({{< relref "execution_model" >}}).

Feel free to skip ahead to the [developer]({{< relref "developer" >}}) 
section if you just want to use it.

## Developer Guide
We have several integrations.

The easiest is to run as a [web service]({{< relref "rpc" >}}) 
that provides synchronization
and account management. 

WarpSync can be used as a dynamic linked library from any language
that supports [FFI]({{< relref "ffi" >}})
C bindings. For an example in JAVA, go to 
this [section]({{< relref "java" >}}).

And finally, if you use rust, WarpSync is a crate that can be incorporated
in your project. You will find an example [here]({{< relref "rust" >}}).
The RustDoc is [here](/doc/warp_api_ffi).

