Getting Started
===============

This tutorial walks you through installing ``cim``, initializing your first
workspace, and running a build. By the end you will have a working SDK
workspace on your machine.

Prerequisites
-------------

**Required:**

- git 2.0+
- make
- tar and unzip
- python3 3.8+, venv and pip
- curl or wget

On Ubuntu, install the dependencies with:

.. code-block:: bash

   sudo apt install -y git make tar unzip python3 python3-pip python3-venv curl wget

**Optional:**

- `Rust 1.56+ <https://rust-lang.org/tools/install/>`_ (to build from source)
- Docker (for containerized development)

Install cim
-----------

``cim`` is distributed as a single binary. Download the
`latest release <https://github.com/analogdevicesinc/cim/releases>`_ for your
platform and place it on your PATH:

.. code-block:: bash

   tar -xzf cim-*.tar.gz
   chmod 711 cim
   cp cim $HOME/.local/bin/

Alternatively, build from source:

.. code-block:: bash

   git clone https://github.com/analogdevicesinc/cim.git
   cd cim
   cargo build --release
   cp target/release/cim $HOME/.local/bin/

Shell completions are available in the ``completions/`` directory of the
repository.

Browse Available Targets
------------------------

Manifests define SDK targets and are stored locally or fetched from git
repositories. List what is available:

.. code-block:: bash

   cim list-targets
   cim list-targets --source https://github.com/<path-to-a>/cim-manifests

To inspect versions for a specific target:

.. code-block:: bash

   cim list-targets -t optee-qemu-v8

Initialize a Workspace
----------------------

.. code-block:: bash

   cim init -t optee-qemu-v8

This creates a workspace at ``$HOME/dsdk-optee-qemu-v8``. Use ``-w`` or
``--workspace`` to pick a different location.

The workspace now contains the cloned repositories, an ``sdk.yml``, and a
``.workspace`` marker file.

Install Dependencies
--------------------

.. code-block:: bash

   cd ~/dsdk-optee-qemu-v8
   cim install os-deps --yes    # system packages (requires sudo)
   cim install toolchains       # cross-compilation toolchains
   cim install pip              # Python packages into .venv

.. tip::

   Pass ``--install`` to ``cim init`` to run all three install steps
   automatically after initialization.

Build and Test
--------------

.. code-block:: bash

   cim makefile         # generate Makefile from sdk.yml
   make sdk-build       # build
   make sdk-test        # test

That's it — you have a fully operational SDK workspace.

Next Steps
----------

- :doc:`/howto/create-manifest` — create your own SDK manifest
- :doc:`/explanation/concepts` — understand workspaces, mirrors, and targets
- :doc:`/reference/cli` — full command reference
