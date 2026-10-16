Getting Started
===============

This tutorial walks you through installing ``cim``, initializing your first
workspace, and running a build. By the end you will have a working project
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

``cim`` is distributed as a single binary. On Linux and macOS, the quickest
way to install the latest release is the install script:

.. code-block:: bash

   curl -fsSL https://analogdevicesinc.github.io/cim/install.sh | sh

It detects your OS and architecture, installs ``cim`` into ``~/.local/bin``,
and tells you whether that directory is on your ``PATH``. Set
``CIM_BIN_DIR`` to install elsewhere, for example system-wide:

.. code-block:: bash

   curl -fsSL https://analogdevicesinc.github.io/cim/install.sh | sudo CIM_BIN_DIR=/usr/local/bin sh

Alternatively, download an archive for your platform from the
`releases page <https://github.com/analogdevicesinc/cim/releases>`_ and copy
the binary onto your ``PATH``:

.. code-block:: bash

   tar -xzf cim-suite-v1.2.4-x86_64-unknown-linux-gnu.tar.gz
   install -m 755 cim-suite-v1.2.4-x86_64-unknown-linux-gnu/cim ~/.local/bin/

Or build from source:

.. code-block:: bash

   git clone https://github.com/analogdevicesinc/cim.git
   cd cim
   cargo build --release
   cp target/release/cim ~/.local/bin/

Check the installation and upgrade later with:

.. code-block:: bash

   cim -v
   cim utils update

Bash completion is available from the source repository:
``./completions/install.sh``.

Browse Available Targets
------------------------

Manifests define targets and are stored locally or fetched from git
repositories. A public manifest repository with open source and Analog Devices
targets is available at https://github.com/joabech/cim-manifests. List what is
available:

.. code-block:: bash

   cim list-targets --source https://github.com/joabech/cim-manifests

To inspect versions for a specific target:

.. code-block:: bash

   cim list-targets --source https://github.com/joabech/cim-manifests -t optee-qemu-v8

Versions are branches or tags of the manifest repository named
``<target>-<version>``, such as ``optee-qemu-v8-v4.9.0``. Pass one to
``cim init --version`` to pin the whole workspace to it; without
``--version``, the manifest's default branch is used.

To avoid typing ``--source`` every time, set it as your default (see
:doc:`/reference/configuration`):

.. code-block:: bash

   cim config --create
   cim config --edit      # set default_source under [sources]

For private manifest repositories, see :doc:`/howto/private-repos`.

Initialize a Workspace
----------------------

.. code-block:: bash

   cim init -t optee-qemu-v8 --source https://github.com/joabech/cim-manifests

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
   cim makefile                 # generate the Makefile from sdk.yml

Targets whose manifest has an ``install`` section also need
``cim install tools --all`` to run those steps; ``optee-qemu-v8`` has none.

.. tip::

   Pass ``--install`` to ``cim init`` to install toolchains and Python
   packages, generate the Makefile, and run the install steps automatically
   after initialization. ``--full`` does the same and installs the OS
   packages first.

Build and Test
--------------

.. code-block:: bash

   make sdk-envsetup    # prepare the build (selects the QEMU v8 build files)
   make sdk-build       # build
   make sdk-test        # test

Add ``-jN`` to run the build in parallel, e.g. ``make -j8 sdk-build``.
``make sdk-envsetup`` is needed once per workspace here because this target's
``build`` phase does not list ``sdk-envsetup`` under ``depends_on``.

That's it — you have a fully operational project workspace.

One-Shot Setup
--------------

``cim bootstrap`` combines the steps: it lets you pick a target and version,
initializes the workspace with ``--install``, and runs the ``envsetup``,
``build`` and ``test`` phases:

.. code-block:: bash

   cim bootstrap --source https://github.com/joabech/cim-manifests

It does not install OS packages. If the target needs some, use
``cim init --full`` instead, or install them before running ``bootstrap``.

Next Steps
----------

- :doc:`/howto/create-manifest` — create your own manifest
- :doc:`/explanation/concepts` — understand workspaces, mirrors, and targets
- :doc:`/reference/cli` — full command reference
