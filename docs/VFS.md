# Virtual File System

The Virtual File System subsystem is intended to organize all of the kernel's filesystems into a single, system-wide object. 
There are two primary consumers of the VFS: page cache, and direct `open/read/write` file operations. Observe that those two
things are not all too different: in order to map a file in page cache, one must first obtain a file descriptor via `open`. 
Alas, page cache is used so often that it has its own special needs for which page cache is optimized. 

## Terminology/Common Structures



## Usage

There are two entrypoints into the VFS. The first is intended by the used by page cache; the VFS can be
treated as a map from `(filesystem, inumber) --> VNode`, where presumably the page cache is storing
what inumber and filesystem each page corresponds to. 

## For Filesystem Implementers

One of the first things you may notice is that everything returns `Arc<dyn VNode>`. That is not a bug. 
This is intended to allow flexible locking structures/implementations. For example, some filesystems
may accept concurrent reads, while other filesystems only want one thread to have read access at a time.
It is strongly recommended to have some sort of lock inside your `VNode` struct, which produces the
same effect as if everything returned `Arc<IntMutex<dyn VNode>>`. 

One of the second things you may notice is that there is no clear distinction between 
files and directories. This is a feature, not a bug. Some filesystems do indeed have
directories that look like their files (for example, the driver filesystem will want
bus devices to act both as directories while also exposing some of their own fuctions).
You are encouraged to draw distinctions between files and directories as you see fit in your 
own code. 

