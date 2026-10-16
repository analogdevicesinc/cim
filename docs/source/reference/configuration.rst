Configuration Reference
=======================

CIM uses a TOML configuration file for user-level settings that apply across
all workspaces. Every setting is optional.

File Location
-------------

- **Unix/Linux/macOS:** ``~/.config/cim/config.toml``
- **Windows:** ``%LOCALAPPDATA%\cim\config.toml``

Create it from the commented template with:

.. code-block:: bash

   cim config --create

Settings
--------

Every setting lives in one of five tables. In TOML, a ``key = value`` line
belongs to the nearest table header above it, so always add a setting under
its own table header.

.. list-table::
   :header-rows: 1

   * - Key
     - Default
     - Description
   * - ``[workspace] mirror``
     - ``$HOME/tmp/mirror``
     - Mirror location for all workspaces. The ``--mirror`` flag takes
       precedence.
   * - ``[workspace] workspace_prefix``
     - ``dsdk-``
     - Prefix for workspace directory names (``$HOME/<prefix><target>``).
   * - ``[workspace] default_workspace``
     - (none)
     - Workspace directory used when ``--workspace`` is not given. It is used
       as the workspace itself, not as a parent directory.
   * - ``[workspace] no_mirror``
     - ``false``
     - Always behave as if ``--no-mirror`` was given.
   * - ``[[workspace.copy_files]]``
     - (none)
     - Personal files (``source``, ``dest``) copied into every workspace, in
       addition to the manifest's ``copy_files``.
   * - ``[sources] default_source``
     - ``$HOME/devel/cim-manifests``
     - Manifest source URL or path, used when ``--source`` is not specified.
   * - ``[[sources.alternate_sources]] url``
     - (none)
     - Additional manifest sources, searched after ``default_source``.
   * - ``[build] documentation_dirs``
     - (none)
     - Comma-separated list of additional documentation directories for
       ``cim docs create``, searched after the default folders (``docs``,
       ``doc``, ``documentation``, ``documents``, the repo root).
   * - ``[build] shell``, ``[build] shell_arg``
     - ``bash``, ``-c``
     - Shell used to run toolchain ``post_install_commands``.
   * - ``[network] cert_validation``
     - ``strict``
     - TLS certificate validation mode: ``strict``, ``relaxed``, or ``auto``.
   * - ``[network] git_timeout_secs``
     - ``900``
     - Hard timeout for every git command.
   * - ``[network] low_speed_limit``, ``[network] low_speed_time_secs``
     - (disabled)
     - Abort a git transfer that stays below ``low_speed_limit`` bytes/s for
       ``low_speed_time_secs`` seconds. Both must be set.
   * - ``[bootstrap] phases``
     - ``["envsetup", "build", "test"]``
     - Phases ``cim bootstrap`` runs after creating the workspace. ``[]`` runs
       none.
   * - ``[bootstrap] jobs``
     - number of CPUs
     - ``-j`` value passed to each ``make sdk-<phase>`` by ``cim bootstrap``.
   * - ``[bootstrap] force``, ``[bootstrap] symlink``
     - ``false``
     - Whether ``cim bootstrap`` passes ``--force`` / ``--symlink`` to init.

Example
-------

.. code-block:: toml

   [workspace]
   mirror = "/fast-ssd/cim-mirror"
   workspace_prefix = "sdk-"

   [sources]
   default_source = "https://github.com/joabech/cim-manifests"

   [[sources.alternate_sources]]
   url = "https://github.com/myteam/custom-manifests"

   [build]
   documentation_dirs = "wiki, manual, reference"

   [network]
   cert_validation = "strict"
   git_timeout_secs = 1800

   [bootstrap]
   phases = ["envsetup", "build", "test"]
   jobs = 8

Managing Configuration
----------------------

.. code-block:: bash

   cim config --create     # Create config file from the template
   cim config --path       # Show the config file location
   cim config --list       # Show current settings
   cim config --get KEY    # Get a specific setting, e.g. workspace.mirror
   cim config --edit       # Open config in editor
   cim config --validate   # Validate config file

Certificate Validation
----------------------

CIM validates TLS certificates by default using strict checking.

.. list-table::
   :header-rows: 1

   * - Mode
     - Behavior
   * - ``strict``
     - Full certificate validation (default).
   * - ``relaxed``
     - Disables certificate validation. **Insecure** — vulnerable to MITM attacks.
   * - ``auto``
     - Tries strict first, falls back to relaxed with a warning.

Set the mode for all commands under ``[network]``, or override it for one
download command:

.. code-block:: bash

   cim install toolchains --cert-validation=relaxed

.. warning::

   Use ``relaxed`` mode only in trusted networks.

Git Timeouts
------------

Every git command ``cim`` runs is killed after ``git_timeout_secs`` (default
900 seconds). Raise it for very large repositories on slow links. Git's own
low-speed abort is disabled by default because large clones can legitimately
stall while git computes deltas; enable it by setting both
``low_speed_limit`` and ``low_speed_time_secs``.
