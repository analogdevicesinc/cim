Manifest Reference
==================

The ``sdk.yml`` manifest defines repositories, build targets, toolchains, and
variables for an SDK target. Two optional companion files,
``os-dependencies.yml`` and ``python-dependencies.yml``, define host packages
and Python dependencies respectively.

.. contents::
   :local:
   :depth: 2

sdk.yml
-------

mirror
~~~~~~

**Required.** Local directory used as a cache for git mirrors, downloaded
toolchains, and ``copy_files`` artifacts. Supports ``$HOME`` expansion.

.. code-block:: yaml

   mirror: $HOME/tmp/mirror

variables
~~~~~~~~~

Key-value map of manifest variables. Values may reference host environment
variables with ``$VAR``.

.. code-block:: yaml

   variables:
     ARCH: arm64
     SDK_BASE_URL: https://artifacts.example.com/sdk

See :doc:`/explanation/variables` for expansion rules.

gits
~~~~

List of git repositories to clone into the workspace.

.. list-table::
   :header-rows: 1

   * - Field
     - Required
     - Description
   * - ``name``
     - yes
     - Directory name in the workspace. Supports nested paths (e.g., ``platform/drivers``).
   * - ``url``
     - yes
     - Git URL (HTTPS or SSH). Supports ``${{ VAR }}`` expansion.
   * - ``commit``
     - yes
     - Branch, tag, or commit hash. Numeric values like ``2025.05`` are accepted.
   * - ``depth``
     - no
     - Shallow clone depth (e.g., ``1`` for a single-commit clone).
   * - ``build``
     - no
     - Per-repo build commands. Generates a Makefile target named after the repo.
   * - ``build_depends_on``
     - no
     - List of repo names that must be built first.
   * - ``git_depends_on``
     - no
     - List of repo names that must be cloned first. Needed for nested repos.
   * - ``documentation_dir``
     - no
     - Custom docs directory within the repo.

The ``build`` field accepts three formats:

.. code-block:: yaml

   # Array
   build:
     - make clean
     - make all

   # Multiline string
   build: |
     make clean
     make all

   # Object with dependencies
   build:
     commands:
       - make all
     depends_on:
       - sdk-envsetup

toolchains
~~~~~~~~~~

List of toolchain archives to download and extract. Triggered by
``cim install toolchains``.

.. list-table::
   :header-rows: 1

   * - Field
     - Required
     - Description
   * - ``url``
     - yes
     - Download URL for the archive.
   * - ``destination``
     - yes
     - Extraction path in the workspace.
   * - ``name``
     - no
     - Archive filename. Derived from URL if omitted.
   * - ``strip_components``
     - no
     - Leading path components to strip (like ``tar --strip-components``).
   * - ``os``
     - no
     - Only install on this OS: ``linux``, ``darwin``, or ``windows``.
   * - ``arch``
     - no
     - Only install on this architecture: ``x86_64``, ``aarch64``, ``arm64``, or ``i386``.
   * - ``sha256``
     - no
     - Expected checksum. Re-downloads if mismatch.
   * - ``mirror_destination``
     - no
     - Custom path in the mirror (defaults to ``destination``).
   * - ``environment``
     - no
     - Env vars for ``post_install_commands``. Supports ``$PWD``, ``$WORKSPACE``, ``$HOME``.
   * - ``post_install_commands``
     - no
     - Commands to run after extraction, executed in ``destination``.

envsetup
~~~~~~~~

Commands that run before build and test targets in the generated Makefile.

.. code-block:: yaml

   envsetup:
     - export PATH=$(pwd)/toolchains/aarch64/bin:$$PATH
     - source /opt/Xilinx/Vivado/2023.2/settings64.sh

build, test, clean, flash
~~~~~~~~~~~~~~~~~~~~~~~~~

Workspace-level commands mapped to ``make sdk-build``, ``make sdk-test``,
``make sdk-clean``, and ``make sdk-flash``. All four accept the same formats
as the ``gits[].build`` field.

copy_files
~~~~~~~~~~

Files to download or copy into the workspace during ``cim init``.

.. list-table::
   :header-rows: 1

   * - Field
     - Required
     - Description
   * - ``source``
     - yes
     - Local path or remote URL.
   * - ``dest``
     - yes
     - Destination in the workspace.
   * - ``cache``
     - no
     - Store in mirror for reuse (``true``/``false``).
   * - ``sha256``
     - no
     - Checksum for integrity verification.
   * - ``symlink``
     - no
     - Symlink from mirror instead of copying (requires ``cache: true``).
   * - ``post_data``
     - no
     - Form data for HTTP POST requests.

install
~~~~~~~

Custom installation targets triggered by ``cim install tools``.

.. list-table::
   :header-rows: 1

   * - Field
     - Required
     - Description
   * - ``name``
     - yes
     - Target name (used with ``cim install tools --list``).
   * - ``commands``
     - no
     - Shell commands to execute.
   * - ``depends_on``
     - no
     - Other install target names that must run first.
   * - ``sentinel``
     - no
     - File path for idempotency. If it exists, the step is skipped.

makefile_include
~~~~~~~~~~~~~~~~

Additional lines to include in the generated Makefile, inserted after variable
definitions and before SDK targets.

.. code-block:: yaml

   makefile_include:
     - "include extra.mk"
     - "CFLAGS += -Wall"

os-dependencies.yml
-------------------

Defines host OS packages per distribution and architecture. Triggered by
``cim install os-deps``.

Top-level keys follow the pattern ``{os}-{arch}`` (e.g., ``linux-x86_64``,
``linux-aarch64``) or just ``{os}`` for backward compatibility. Each contains
distribution entries keyed as ``{distro}-{version}`` (e.g., ``ubuntu-22.04``).

Each distribution entry has:

- ``command``: the install command (e.g., ``apt-get install``, ``dnf install``, ``brew install``)
- ``packages``: list of package names

.. code-block:: yaml

   common_deps: &common
     - build-essential
     - git

   linux-x86_64:
     ubuntu-22.04:
       command: "apt-get install"
       packages: *common

   macos:
     macos-any:
       command: "brew install"
       packages:
         - cmake
         - git

python-dependencies.yml
-----------------------

Defines Python packages organized into profiles. Triggered by
``cim install pip``. Packages are installed into the workspace ``.venv``.

.. code-block:: yaml

   profiles:
     minimal:
       packages: []
     default:
       packages:
         - numpy
     dev:
       packages:
         - numpy
         - pytest

   default: default

The ``default`` key at the root determines which profile is used when no
``--profile`` flag is specified.
