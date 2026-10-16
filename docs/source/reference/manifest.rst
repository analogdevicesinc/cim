Manifest Reference
==================

The ``sdk.yml`` manifest defines repositories, build targets, toolchains, and
variables for a target. Two optional companion files,
``os-dependencies.yml`` and ``python-dependencies.yml``, define host packages
and Python dependencies respectively.

.. contents::
   :local:
   :depth: 2

sdk.yml
-------

Every section is optional. Unknown top-level keys are ignored, including the
``mirror`` key of older manifests: the mirror location is a per-user setting
(see :doc:`/reference/configuration`).

YAML anchors and aliases (``&name`` / ``*name``) can be used to share content
within a file.

variables
~~~~~~~~~

Key-value map of manifest variables. Values may reference host environment
variables with ``$VAR``, and workspace paths with ``${{ WORKSPACE }}``.

.. code-block:: yaml

   variables:
     ARCH: arm64
     SDK_BASE_URL: https://artifacts.example.com/sdk
     PREFIX: ${{ WORKSPACE }}/.local

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
     - Branch, tag, or commit hash. A branch is checked out at its latest
       commit; tags and hashes pin the repository. Numeric values like
       ``2025.05`` are accepted.
   * - ``build``
     - no
     - Per-repo build commands, as a list or a multi-line string. Generates a
       Makefile target named after the repo.
   * - ``build_depends_on``
     - no
     - Targets that must be built first: repo names, ``sdk-<phase>`` or
       ``install-<name>``. ``depends_on`` is accepted as an alias.
   * - ``git_depends_on``
     - no
     - Repos that must be cloned first. Needed for nested repos.
   * - ``python-deps``
     - no
     - Requirements file(s), relative to the repo, installed into
       ``.cim/<name>/.venv`` by ``cim install pip``.
   * - ``group``
     - no
     - Group name or list of names (default: ``default``). See
       :doc:`/howto/repo-groups`.
   * - ``documentation_dir``
     - no
     - Docs directory within the repo for ``cim docs create``. It is searched
       after the default folders (``docs``, ``doc``, ``documentation``,
       ``documents``, the repo root) and ``[build] documentation_dirs``, so it
       only applies when none of those contain documentation.

The ``build`` field accepts two formats:

.. code-block:: yaml

   gits:
     - name: myproject
       url: https://github.com/myorg/myproject.git
       commit: main
       # List
       build:
         - $(MAKE) -C myproject clean
         - $(MAKE) -C myproject all

     - name: mylib
       url: https://github.com/myorg/mylib.git
       commit: v1.2.0
       # Multi-line string, one command per line
       build: |
         $(MAKE) -C mylib clean
         $(MAKE) -C mylib all

toolchains
~~~~~~~~~~

List of toolchain archives to download and extract. Installed by
``cim install toolchains`` (and by ``cim init --install``/``--full``).

.. list-table::
   :header-rows: 1

   * - Field
     - Required
     - Description
   * - ``url``
     - yes
     - Download URL for the archive. If it does not end with ``name``,
       ``name`` is appended to it.
   * - ``destination``
     - yes
     - Extraction path, relative to the workspace.
   * - ``name``
     - no
     - Archive filename. Derived from the URL if omitted.
   * - ``strip_components``
     - no
     - Leading path components to strip (like ``tar --strip-components``).
   * - ``os``
     - no
     - Only install on this OS: ``linux``, ``darwin``, or ``windows``.
   * - ``arch``
     - no
     - Only install on this architecture: ``x86_64``, ``arm64``, or ``i386``.
   * - ``sha256``
     - no
     - Expected checksum. Re-downloads if mismatch.
   * - ``mirror_destination``
     - no
     - Custom path in the mirror for ``--symlink`` installs.
   * - ``environment``
     - no
     - Env vars for ``post_install_commands``. Supports ``$PWD``, ``$WORKSPACE``, ``$HOME``.
   * - ``post_install_commands``
     - no
     - Commands to run after extraction, executed in ``destination``.
   * - ``headers``, ``basic_auth``
     - no
     - Authentication for the download (see ``copy_files``).

Supported formats are ``.tar.xz``, ``.tar.gz``, ``.tar`` and ``.zip``; other
files (such as installer scripts) are copied into ``destination``. See
:doc:`/howto/manage-toolchains` for examples.

Phases: envsetup, build, test, clean, flash, help
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Workspace-level commands mapped to ``make sdk-envsetup``, ``make sdk-build``,
``make sdk-test``, ``make sdk-clean``, ``make sdk-flash`` and
``make sdk-help``. Each phase takes ``commands`` and an optional
``depends_on`` list of Makefile targets (``sdk-<phase>``, repo names, or
``install-<name>``) that must run first:

.. code-block:: yaml

   build:
     commands: |
       $(MAKE) -C myproject
       @echo "Build done"
     depends_on:
       - sdk-envsetup

   flash:
     commands:
       - "@echo \"Copy myproject/out/image.bin to your target\""

   test:
     depends_on:
       - myproject

``commands`` is either a multi-line string (one command per line) or a list.
A phase may also consist of ``depends_on`` only. The legacy form, a plain list
or string directly under the phase (``build: [...]``), still works but cannot
declare ``depends_on``.

Rules for commands:

- Each command is one recipe line, run by Make in its own ``/bin/sh``.
  Environment changes (``export``, ``source``, ``cd``) do not carry over to
  the next line or to other phases.
- Make sees the command first: write ``$$`` for a shell ``$`` (``$$HOME``,
  ``$$(nproc)``), and use ``$(MAKE)`` for sub-makes.
- ``${{ VAR }}`` becomes ``$(VAR)``.
- A leading ``@`` (do not echo) or ``-`` (ignore errors) is passed to Make. In
  a YAML list, quote such items (``- "@echo hi"``): ``@`` cannot start a plain
  YAML value. Inside a ``|`` block no quoting is needed.
- No ``-j`` is added; users pick parallelism with ``make -jN``.

.. _envsetup:

envsetup
~~~~~~~~

``sdk-envsetup`` runs only when invoked directly or listed in another phase's
``depends_on``. Because every command runs in its own shell, ``envsetup``
cannot set environment variables for later phases. Use it for checks and
preparation instead:

.. code-block:: yaml

   envsetup:
     commands: |
       @command -v vivado >/dev/null || { echo "Run: . ./env.sh first"; exit 1; }

   build:
     commands: |
       $(MAKE) -C hdl/projects/fmcomms2/zed
     depends_on:
       - sdk-envsetup

When users need a shell environment (for example a vendor tool's settings
script), ship a script with the target and copy it into the workspace with
``copy_files``; users source it once per shell before running ``make``:

.. code-block:: yaml

   copy_files:
     - source: env.sh      # targets/<name>/env.sh in the manifest repository
       dest: env.sh

Pass tool locations that the manifest controls, such as toolchains in the
workspace, through variables instead (see
:doc:`/tutorials/linux-kernel`).

phases
~~~~~~

Extra phase names. Each one gets an ``sdk-<phase>`` target in addition to the
standard six:

.. code-block:: yaml

   phases:
     - deploy
     - lint

Custom phases are implemented in per-repository Makefile fragments (see
``build_folder``): a ``<repo>-<phase>`` target in ``build/<repo>.mk``, e.g.
``myrepo-deploy``, becomes a prerequisite of ``sdk-deploy``.

copy_files
~~~~~~~~~~

Files to download or copy into the workspace during ``cim init`` (and
``cim utils sync-copy-files``).

.. list-table::
   :header-rows: 1

   * - Field
     - Required
     - Description
   * - ``source``
     - yes
     - Remote URL, or local path. Relative paths are resolved against the
       target directory in the manifest repository. Globs and directories are
       copied recursively.
   * - ``dest``
     - yes
     - Destination, relative to the workspace root.
   * - ``cache``
     - no
     - Store downloads in the mirror for reuse (``true``/``false``).
   * - ``symlink``
     - no
     - Symlink from the mirror instead of copying (requires ``cache: true``).
   * - ``sha256``
     - no
     - Checksum for integrity verification.
   * - ``post_data``
     - no
     - Form data for HTTP POST requests.
   * - ``headers``
     - no
     - Map of HTTP header names to values, e.g.
       ``Authorization: "Bearer $MY_API_TOKEN"``. Values may reference host
       environment variables; an unset variable is an error.
   * - ``basic_auth``
     - no
     - ``"user:password"`` for HTTP Basic auth, with the same ``$VAR``
       expansion.

install
~~~~~~~

Custom installation steps. Each becomes an ``install-<name>`` Makefile target;
``install-all`` runs them all. Run them with ``cim install tools`` or ``make``;
``cim init --install`` runs ``install-all``.

.. list-table::
   :header-rows: 1

   * - Field
     - Required
     - Description
   * - ``name``
     - yes
     - Step name (``cim install tools NAME``, ``make install-NAME``).
   * - ``commands``
     - no
     - Shell commands, as a list or a multi-line string.
   * - ``depends_on``
     - no
     - Other install step names (without ``install-``) that must run first.
   * - ``sentinel``
     - no
     - ``true`` to run the step only once: on success CIM creates
       ``.cim/<name>-installed`` and skips the step while it exists.
   * - ``depends_on_gits``
     - no
     - Repos the step needs. If one is excluded from the workspace, the step
       is dropped from the Makefile.

With ``sentinel: true``, all commands of the step run in one shell, joined
with ``&&``; do not prefix them with ``@`` or ``-``. Commands containing
``cd`` run in a subshell, so the next command starts in the workspace root
again. ``cim install tools NAME --force`` removes the sentinel and runs the
step again.

.. code-block:: yaml

   copy_files:
     - source: https://github.com/protocolbuffers/protobuf/releases/download/v21.7/protoc-21.7-linux-x86_64.zip
       dest: downloads/protoc.zip
       cache: true

   install:
     - name: protoc
       sentinel: true
       commands:
         - mkdir -p opt/protoc
         - cd opt/protoc && unzip -q -o ../../downloads/protoc.zip
         - opt/protoc/bin/protoc --version

makefile_include and build_folder
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``cim makefile`` adds ``-include build/<repo>.mk`` for every repository
fragment found in ``build_folder`` (default: ``build``). ``makefile_include``
adds explicit lines before those, and can suppress fragments:

.. code-block:: yaml

   makefile_include:
     files:
       - include extra.mk
     exclude:
       - qemu            # do not include build/qemu.mk

The legacy form is a plain list of lines
(``makefile_include: ["include extra.mk"]``).

direnv
~~~~~~

.. code-block:: yaml

   direnv:
     used: true
     venv_path: .venv

When ``used`` is ``true`` and `direnv <https://direnv.net>`_ is installed,
``cim init`` writes an ``.envrc`` that creates and activates the workspace
virtual environment, and allows it.

extends and overlay
~~~~~~~~~~~~~~~~~~~

Build a target on top of another one. See :doc:`/howto/compose-manifests`.

os-dependencies.yml
-------------------

Defines host OS packages per distribution and architecture. Installed by
``cim install os-deps`` (and ``cim init --full``).

Top-level keys follow the pattern ``{os}-{arch}`` (e.g., ``linux-x86_64``,
``linux-aarch64``) or just ``{os}`` (``linux``, ``macos``, ``windows``) as a
fallback. Each contains distribution entries keyed as ``{distro}-{version}``
from ``/etc/os-release`` (e.g., ``ubuntu-24.04``), falling back to
``{distro}``; on macOS use ``macos-any``.

Each distribution entry has:

- ``command``: the install command (e.g., ``apt-get install``, ``dnf install``, ``brew install``)
- ``packages``: list of package names, or a list of lists that CIM flattens and
  de-duplicates

.. code-block:: yaml

   common_deps: &common
     - build-essential
     - git

   linux-x86_64:
     ubuntu-24.04:
       command: "apt-get install"
       packages: *common

   macos:
     macos-any:
       command: "brew install"
       packages:
         - cmake
         - git

Top-level entries that are not OS keys, like ``common_deps`` above, are
ignored, so they can hold anchors.

python-dependencies.yml
-----------------------

Defines Python packages organized into profiles. Installed by
``cim install pip`` into the workspace ``.venv``.

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
       requirements:
         - myproject/requirements-dev.txt

   default: default

Each profile has ``packages`` (pip requirement specifiers, passed to pip from
the workspace root) and/or ``requirements`` (requirements files, relative to
the workspace root). The ``default`` key at the root determines which profile
is used when no ``--profile`` flag is specified; it defaults to ``docs``.
