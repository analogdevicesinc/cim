Build System Integration
========================

CIM does not replace your project's build system. Instead, it generates a
thin Makefile that wires together the different build systems used across
repositories in a workspace.

How Makefile Generation Works
-----------------------------

When you run ``cim makefile``, CIM reads ``sdk.yml`` and produces a Makefile
with:

1. **Variable definitions** — each entry in ``variables`` becomes a weak
   assignment (``VAR ?= value``), so environment variables and command-line
   overrides take precedence.

2. **makefile_include lines** — inserted after variables, before targets.

3. **SDK targets** — standard entry points that delegate to the underlying
   build systems:

   - ``sdk-envsetup`` — runs ``envsetup`` commands
   - ``sdk-build`` — runs workspace-level ``build`` commands
   - ``sdk-test`` — runs ``test`` commands
   - ``sdk-clean`` — runs ``clean`` commands
   - ``sdk-flash`` — runs ``flash`` commands

4. **Per-repo targets** — each git repo with a ``build`` field gets its own
   target, named after the repo. Dependencies declared via
   ``build_depends_on`` become Makefile prerequisites.

Example
~~~~~~~

Given this manifest:

.. code-block:: yaml

   variables:
     ARCH: arm64

   envsetup:
     - export PATH=$(pwd)/toolchains/bin:$$PATH

   build:
     - $(MAKE) -C linux ARCH=${{ ARCH }}

   gits:
     - name: linux
       url: https://github.com/example/linux.git
       commit: main
       build:
         - $(MAKE) -C linux -j$$(nproc)

     - name: rootfs
       url: https://github.com/example/rootfs.git
       commit: main
       build_depends_on:
         - linux
       build:
         - $(MAKE) -C rootfs

The generated Makefile will contain:

.. code-block:: makefile

   ARCH ?= arm64

   sdk-envsetup:
   	export PATH=$(pwd)/toolchains/bin:$$PATH

   sdk-build: sdk-envsetup
   	$(MAKE) -C linux ARCH=$(ARCH)

   linux: sdk-envsetup
   	$(MAKE) -C linux -j$$(nproc)

   rootfs: linux
   	$(MAKE) -C rootfs

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

**envsetup runs first.** The ``sdk-envsetup`` target is a prerequisite of
``sdk-build`` and ``sdk-test``, ensuring the environment (PATH, vendor tool
scripts) is configured before any build commands execute.

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
build.
