# GitHub Runners
## How to Run Your Own
Set up a [GitHub
runner](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/add-runners)
however you like. Ask Flowyent for a runner token. Make sure the
runner environment has an installation of
[Nix](https://nixos.org/download/) (preferrably Multi-user) and
[Direnv](https://direnv.net/docs/installation.html).

## Known Issues
Every single time the action builds the kernel, it does it from
scratch. This means buildtool, the kernel, and their dependencies.
This could be optimized with further work.
