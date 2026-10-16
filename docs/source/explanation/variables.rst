Variable Expansion
==================

CIM has two distinct variable expansion mechanisms that run at different times.
Understanding when each one fires is key to writing correct manifests.

Host Environment Variables
--------------------------

Standard ``$VAR`` or ``${VAR}`` references in variable **values** are resolved
when CIM loads the manifest. This lets you pull in host-specific paths or
settings:

.. code-block:: yaml

   variables:
     BUILD_USER: $USER             # resolved from the host environment
     TOOLS_DIR: ${HOME}/tools      # resolved to e.g. /home/user/tools

If the host variable is not set, the literal ``$VAR`` text remains and CIM
prints a warning.

Manifest Variables
------------------

References using the ``${{ VAR }}`` syntax are **not** resolved at load time.
Instead, they are emitted as ``$(VAR)`` in the generated Makefile, so Make
expands them at recipe time:

.. code-block:: yaml

   variables:
     ARCH: arm64
     CROSS_COMPILE: aarch64-none-linux-gnu-

   build:
     commands: |
       $(MAKE) -C linux ARCH=${{ ARCH }} CROSS_COMPILE=${{ CROSS_COMPILE }}

In the generated Makefile this becomes:

.. code-block:: makefile

   ARCH ?= arm64
   CROSS_COMPILE ?= aarch64-none-linux-gnu-

   sdk-build:
   	$(MAKE) -C linux ARCH=$(ARCH) CROSS_COMPILE=$(CROSS_COMPILE)

Because ``?=`` is a weak assignment, users can override at build time without
editing the manifest:

.. code-block:: bash

   make ARCH=arm CROSS_COMPILE=arm-none-linux-gnueabihf- sdk-build

Variables given on the ``make`` command line are also passed on to every
sub-make, as if they had been typed there. Environment variables override
``?=`` assignments too, but are not forwarded as command-line variables. Use
them when a sub-build reacts to command-line variables (for example the ADI
HDL build scripts):

.. code-block:: bash

   ARCH=arm CROSS_COMPILE=arm-none-linux-gnueabihf- make sdk-build

``${{ VAR }}`` also works inside variable values, which is useful for paths
inside the workspace:

.. code-block:: yaml

   variables:
     PREFIX: ${{ WORKSPACE }}/.local    # becomes PREFIX ?= $(WORKSPACE)/.local

Variables Provided by the Makefile
----------------------------------

The generated Makefile always defines these variables, so commands can use
them with ``${{ VAR }}`` or ``$(VAR)``:

- ``WORKSPACE`` — absolute path of the workspace root
- ``<NAME>_DIR`` — absolute path of each repository, e.g. ``LINUX_DIR`` for
  a repository named ``linux`` (``-`` and ``.`` become ``_``)
- ``<NAME>_VENV`` — the per-repository virtual environment of repositories
  that declare ``python-deps``

Dollar Signs in Commands
------------------------

Commands are copied into the Makefile as they are, so Make sees every ``$``
first. Write ``$$`` when the shell should see a ``$``:

.. code-block:: yaml

   test:
     commands: |
       @echo "Running tests as $$USER on $$(uname -m)"

``$(pwd)`` or ``$HOME`` written with a single ``$`` are Make references
(``$(pwd)`` is an empty Make variable), not shell expansions. Use
``$(WORKSPACE)`` for the workspace root.

Expansion Context Table
-----------------------

.. list-table::
   :header-rows: 1

   * - Context
     - Syntax
     - Expanded By
   * - Variable values
     - ``$VAR``, ``${VAR}``
     - CIM at load time
   * - Git URLs
     - ``${{ VAR }}``
     - CIM at load time
   * - Toolchain URLs, names, destinations
     - ``${{ VAR }}``
     - CIM at load time
   * - ``copy_files`` source/dest
     - ``${{ VAR }}``
     - CIM at load time
   * - Build/test/clean/flash/envsetup/install commands, variable values
     - ``${{ VAR }}``
     - Make at recipe time (via ``$(VAR)``)
   * - Toolchain ``environment`` values and ``post_install_commands``
     - ``$PWD``, ``$WORKSPACE``, ``$HOME``, host ``$VAR``
     - CIM at install time

Why Two Mechanisms?
-------------------

The split exists because build commands need to be overridable at ``make``
time, not locked in when the manifest is loaded. If ``${{ ARCH }}`` were
resolved immediately, there would be no way to run
``make ARCH=arm sdk-build`` — the value would already be baked into the
Makefile.

For git URLs and toolchain URLs, immediate resolution is correct because those
values are consumed by CIM during ``init`` and ``install``, before any Makefile
exists.

Unresolved References
---------------------

In git URLs and toolchain or ``copy_files`` fields, ``${{ VAR }}`` references
to unknown variables are left unchanged, which makes the mistake visible in
the resulting error. In commands they become ``$(VAR)``, which Make expands to
an empty string unless ``VAR`` is set in the environment or on the ``make``
command line.
