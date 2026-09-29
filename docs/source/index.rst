Code in Motion
===============

Code in Motion (``cim``) manages multi-repository SDK workspaces and makes it
possible to bundle everything needed to setup, build and work with software
projects of any size.

With the concept of "manifests", it allows you to create dedicated manifest
repositories for all sorts of projects. A single init command replaces manual
README instructions and copy-paste command workflows.

What Problems Does This Solve?
------------------------------

- **Automated setup** — replace manual README instructions and copy-paste
  command workflows with a single ``cim init`` command
- **Repository synchronization** — clone and update multiple git repos to
  exact commits specified in a manifest
- **Toolchain management** — download and extract cross-platform toolchains
  with automatic OS and architecture filtering
- **Reproducible builds** — lock all dependencies to specific versions for
  consistent environments across machines and CI
- **Offline operation** — mirror repos and artifacts locally to save
  bandwidth, disk space, and setup time
- **Workspace isolation** — everything lives in the workspace folder except
  host OS dependencies and the shared mirror. Deleting the workspace deletes
  it all.

Quick Example
-------------

Setting up and building an entire project takes just a handful of commands:

.. code-block:: bash

   cim list-targets                          # browse available SDKs
   cim init -t my-sdk --install              # clone repos, install toolchains
   cd ~/dsdk-my-sdk
   cim makefile                              # generate Makefile from manifest
   make sdk-build                            # build
   make sdk-test                             # test

The tool standardizes build targets across projects (``sdk-build``,
``sdk-test``, ``sdk-clean``, ``sdk-flash``), so teams don't need to learn
different conventions for each software project. Advanced users can continue
working with the underlying build systems directly when needed.

Key Features
------------

- **Single binary** — no installer, no runtime dependencies beyond git and
  make. Download and go.
- **Manifest-driven** — YAML manifests define repos, toolchains, dependencies,
  and build commands. Share them via git.
- **Cross-platform** — runs on Linux (x86_64, aarch64), macOS, and Windows.
  Toolchains are filtered by OS and architecture automatically.
- **CI/CD ready** — works in GitHub Actions, Jenkins, and other CI systems
  out of the box.
- **Mirror and cache** — shared local mirror avoids redundant downloads across
  workspaces and enables offline builds.

.. toctree::
   :maxdepth: 1
   :caption: Tutorials

   tutorials/getting-started
   tutorials/linux-kernel
   tutorials/fpga-hdl
   tutorials/python-library

.. toctree::
   :maxdepth: 1
   :caption: How-to Guides

   howto/create-manifest
   howto/manage-toolchains
   howto/manage-dependencies
   howto/use-docker

.. toctree::
   :maxdepth: 1
   :caption: Reference

   reference/cli
   reference/manifest
   reference/configuration

.. toctree::
   :maxdepth: 1
   :caption: Explanation

   explanation/concepts
   explanation/variables
   explanation/build-system

.. toctree::
   :maxdepth: 1
   :caption: Development

   contributing
