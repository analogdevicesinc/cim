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
     SDK_ARCH: $HOST_ARCH          # resolved from the host environment
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
     - $(MAKE) -C linux ARCH=${{ ARCH }} CROSS_COMPILE=${{ CROSS_COMPILE }}

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
   * - Build/test/clean/flash/envsetup commands
     - ``${{ VAR }}``
     - Make at recipe time (via ``$(VAR)``)
   * - Toolchain ``environment`` values
     - ``$PWD``, ``$WORKSPACE``, ``$HOME``
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

Unresolved ``${{ VAR }}`` references are left unchanged and a warning is
printed. This makes mistakes visible rather than silently ignored.
