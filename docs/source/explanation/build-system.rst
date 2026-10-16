Build System Integration
========================

CIM does not replace your project's build system. Instead, it generates a
thin Makefile that wires together the different build systems used across
repositories in a workspace.

How Makefile Generation Works
-----------------------------

When you run ``cim makefile`` (or ``cim init --install``), CIM reads
``sdk.yml`` and produces a Makefile with:

1. **Workspace variables** — ``WORKSPACE`` (the workspace root) and a
   ``<NAME>_DIR`` variable per repository (e.g. ``LINUX_DIR``). Repositories
   with ``python-deps`` also get a ``<NAME>_VENV`` variable.

2. **Manifest variables** — each entry in ``variables`` becomes a weak
   assignment (``VAR ?= value``), so environment variables and command-line
   overrides take precedence.

3. **Includes** — ``makefile_include`` entries, followed by any per-repository
   fragments (``build/<name>.mk``) found in the ``build_folder``.

4. **Phase targets** — standard entry points that delegate to the underlying
   build systems:

   - ``sdk-envsetup`` — runs ``envsetup`` commands
   - ``sdk-build`` — runs workspace-level ``build`` commands
   - ``sdk-test`` — runs ``test`` commands
   - ``sdk-clean`` — runs ``clean`` commands
   - ``sdk-flash`` — runs ``flash`` commands
   - ``sdk-help`` — lists the available targets (or runs ``help`` commands)

   Custom phases listed under ``phases`` get an ``sdk-<phase>`` target too.

5. **Install targets** — ``install-<name>`` for each ``install`` entry, plus
   ``install-all``.

6. **Per-repo targets** — each git repo with a ``build`` field gets its own
   target, named after the repo. Repos listed in ``build_depends_on`` become
   Makefile prerequisites.

Example
~~~~~~~

Given this manifest (the ``myorg`` repository URLs are placeholders):

.. code-block:: yaml

   variables:
     ARCH: arm64

   envsetup:
     commands: |
       @command -v bc >/dev/null || { echo "bc is required to build the kernel"; exit 1; }

   build:
     depends_on:
       - sdk-envsetup
       - rootfs

   gits:
     - name: linux
       url: https://github.com/myorg/linux.git
       commit: main
       build:
         - $(MAKE) -C linux ARCH=${{ ARCH }}

     - name: rootfs
       url: https://github.com/myorg/rootfs.git
       commit: main
       build_depends_on:
         - linux
       build:
         - $(MAKE) -C rootfs

The generated Makefile contains (comments, ``.PHONY`` lines and the unused
phase targets omitted):

.. code-block:: makefile

   WORKSPACE := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))

   LINUX_DIR := $(WORKSPACE)/linux
   ROOTFS_DIR := $(WORKSPACE)/rootfs

   ARCH ?= arm64

   sdk-envsetup:
   	@command -v bc >/dev/null || { echo "bc is required to build the kernel"; exit 1; }

   sdk-build: sdk-envsetup rootfs

   linux:
   	$(MAKE) -C linux ARCH=$(ARCH)

   rootfs: linux
   	$(MAKE) -C rootfs

``make sdk-build`` runs the ``bc`` check, then builds ``linux`` and finally
``rootfs``. ``build`` has no commands of its own here: a phase may consist of
``depends_on`` only.

Design Decisions
----------------

**Thin wrapper, not a build system.** The generated Makefile should contain
minimal logic. Complex build steps belong in the repository's own build
system. The manifest wires things together; it does not replicate build
scripts.

**Weak variable assignments.** Using ``?=`` means the Makefile respects
environment variables and command-line overrides. Users can customize builds
without editing the manifest:

.. code-block:: bash

   make ARCH=arm sdk-build

**Dependency ordering via Make.** Rather than implementing its own dependency
graph, CIM relies on Make's built-in prerequisite system. This is transparent
and familiar to developers.

**Explicit ordering.** No phase depends on another one automatically. If
``sdk-build`` needs ``sdk-envsetup`` (or a repo target, or an
``install-<name>`` target) to run first, list it under ``depends_on``.

**One shell per command.** Every command becomes its own recipe line, and Make
runs each line in a separate ``/bin/sh``. ``cd``, ``export`` and ``source``
therefore do not carry over to the next command, or from ``sdk-envsetup`` to
``sdk-build``. Use ``envsetup`` for checks and one-time preparation, chain
dependent commands on one line (``cd dir && ./configure``), and pass tool
locations through variables. See :ref:`envsetup` for ways to provide a shell
environment.

**Parallelism is the user's choice.** The generated Makefile never adds a
``-j`` flag. Use ``$(MAKE)`` for sub-makes so they share Make's jobserver, and
pick the job count when you invoke Make:

.. code-block:: bash

   make -j8 sdk-build

``cim bootstrap`` passes ``-j<jobs>`` to each phase, using ``jobs`` under
``[bootstrap]`` in the configuration file (default: the number of CPUs).

When to Use Per-Repo vs Workspace-Level Targets
------------------------------------------------

- **Per-repo targets** (``gits[].build``) are useful when you want to build
  individual repositories independently, or when repositories have explicit
  dependencies on each other.

- **Workspace-level targets** (``build``, ``test``, ``clean``, ``flash``) are
  the main entry points. They are what end users run via
  ``make sdk-build``.

Both can coexist. A common pattern is to define per-repo targets for
fine-grained control and workspace-level targets that orchestrate the full
build, as in the example above.
