# Device File System

The Device File System is the principal way that users will interact with the 
various device drivers on the system (and ergo, the various devices)

## Usage (Users)

DevFS is only accessible by starting from the root of the VFS, and traversing
to one of the files/subdirectories under `/dev`. Users interact with `/dev` as
they would with any file on a Linux system.  