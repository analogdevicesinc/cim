CLI Reference
=============

Run ``cim <command> --help`` for the authoritative list of options of the
installed version. ``cim -v`` prints the version of ``cim`` itself.

list-targets
------------

Show available targets from a manifest repository.

.. code-block:: bash

   cim list-targets [--source URL|PATH] [--target NAME] [--verbose]
                    [--format text|json]

With ``--target``, lists the versions of that target: the branches and tags of
the manifest repository whose names start with ``<target>-``. Pass the full
name to ``cim init --version``.

init
----

Initialize a workspace from a target.

.. code-block:: bash

   cim init --target NAME [--source URL|PATH] [--version VERSION]
            [--workspace DIR] [--match REGEX]
            [--include-group NAMES] [--exclude-group NAMES]
            [--install | --full] [--no-sudo] [--symlink]
            [--no-mirror] [--mirror DIR] [--force] [--yes] [--verbose]

.. list-table::
   :header-rows: 1

   * - Option
     - Description
   * - ``--target``, ``-t``
     - Target name from the manifest repository (required)
   * - ``--source``, ``-s``
     - Manifest repository: a git URL (``https://``, ``http://`` or
       ``git@host:...``) or a local path. Defaults to ``default_source`` from
       the configuration file, then ``$HOME/devel/cim-manifests``.
   * - ``--version``, ``-v``
     - Branch or tag of the manifest repository to use, e.g.
       ``optee-qemu-v8-v4.9.0`` (see ``list-targets --target``)
   * - ``--workspace``, ``-w``
     - Workspace directory (default: ``$HOME/dsdk-<target>``)
   * - ``--match``
     - Only clone repos whose name matches the regex (comma-separated values
       are alternatives)
   * - ``--include-group``, ``--exclude-group``
     - Only clone / skip repos in these comma-separated groups (see
       :doc:`/howto/repo-groups`)
   * - ``--install``
     - After cloning: install toolchains and Python packages, generate the
       Makefile, and run ``make install-all`` if the manifest has ``install``
       steps
   * - ``--full``
     - Like ``--install``, but installs OS packages first (uses sudo unless
       ``--no-sudo``)
   * - ``--no-sudo``
     - Run the OS package manager without sudo (with ``--full``)
   * - ``--symlink``
     - Install toolchains and the Python venv into the mirror and symlink
       them into the workspace
   * - ``--no-mirror``
     - Clone directly from the remotes, without the mirror
   * - ``--mirror``
     - Mirror directory for this invocation
   * - ``--force``
     - Remove an existing workspace and create it again
   * - ``--yes``, ``-y``
     - Skip confirmation prompts

Plain ``cim init`` does not generate a Makefile; run ``cim makefile`` or use
``--install``/``--full``.

bootstrap
---------

Pick a target and version, initialize the workspace, and run its build phases
in one command.

.. code-block:: bash

   cim bootstrap [--target NAME] [--source URL|PATH] [--version VERSION]
                 [--workspace DIR] [--match REGEX]
                 [--include-group NAMES] [--exclude-group NAMES]
                 [--no-mirror] [--mirror DIR] [--yes] [--verbose]

Without ``--target`` (or ``--version``), ``cim`` shows an interactive list to
pick from; this requires a terminal, so pass the options explicitly in CI.
``bootstrap`` runs the equivalent of ``cim init --install`` and then
``make sdk-<phase> -j<jobs>`` for each phase, stopping at the first failure.
The phases (default: ``envsetup``, ``build``, ``test``), the job count, and
whether ``--force``/``--symlink`` are used come from the ``[bootstrap]``
section of the configuration file (see :doc:`/reference/configuration`).

``bootstrap`` never installs OS packages. Run ``cim install os-deps`` (or
``cim init --full``) once beforehand if the target needs them.

update
------

Update the repositories in the workspace to the commits in ``sdk.yml``.

.. code-block:: bash

   cim update [--match REGEX] [--include-group NAMES] [--exclude-group NAMES]
              [--all] [--no-mirror] [--mirror DIR] [--verbose]

By default the repository selection stored by ``cim init`` is reused.
``--all`` updates every repository and clears the stored selection.
``cim update`` does not regenerate the Makefile; run ``cim makefile`` after
changing ``sdk.yml``.

makefile
--------

Generate the workspace ``Makefile`` (and ``.vscode/tasks.json``) from
``sdk.yml``. Must be run from within a workspace.

.. code-block:: bash

   cim makefile [--no-dividers] [--include-group NAMES] [--exclude-group NAMES]

See :doc:`/explanation/build-system` for what the Makefile contains.

foreach
-------

Run a shell command in each repository in the workspace.

.. code-block:: bash

   cim foreach "COMMAND" [--match REGEX] [--include-group NAMES]
               [--exclude-group NAMES]

Example:

.. code-block:: bash

   cim foreach "git status"
   cim foreach "git log --oneline -5" --match "linux,hdl"

add
---

Add a git repository to the workspace ``sdk.yml``.

.. code-block:: bash

   cim add --name NAME --url URL --commit COMMIT

install
-------

os-deps
~~~~~~~

Install system packages from ``os-dependencies.yml``. CIM detects the host OS
and distribution automatically.

.. code-block:: bash

   cim install os-deps [--yes] [--no-sudo]

pip
~~~

Install Python packages from ``python-dependencies.yml`` into the workspace
``.venv``, and each repository's ``python-deps`` into ``.cim/<name>/.venv``.

.. code-block:: bash

   cim install pip [--profile PROFILE[,PROFILE]] [--list-profiles] [--force]
                   [--symlink] [--include-group NAMES] [--exclude-group NAMES]
   cim install pip --repair

Example:

.. code-block:: bash

   cim install pip --profile dev,docs

toolchains
~~~~~~~~~~

Download and extract the toolchains in ``sdk.yml`` that match the host.

.. code-block:: bash

   cim install toolchains [--symlink] [--force] [--verbose]

tools
~~~~~

Run the install steps defined in the ``install`` section of ``sdk.yml``. This
wraps ``make install-<name>`` / ``make install-all``, so the Makefile must
exist (``cim makefile``).

.. code-block:: bash

   cim install tools --list           # list install steps
   cim install tools NAME             # run one step
   cim install tools NAME --force     # run it again, ignoring its sentinel
   cim install tools --all            # run every step

docs
----

create
~~~~~~

Aggregate documentation from the repositories in the workspace into
``<workspace>/documentation``.

.. code-block:: bash

   cim docs create [--force] [--theme THEME] [--symlink] [--verbose]

build
~~~~~

Build the aggregated documentation with Sphinx.

.. code-block:: bash

   cim docs build [--format FORMAT]    # default: html

Sphinx and the theme (default ``sphinx_rtd_theme``) must be installed in the
workspace ``.venv``, for example through a ``docs`` profile in
``python-dependencies.yml`` with ``sphinx`` and ``sphinx-rtd-theme``.

serve
~~~~~

Serve the built documentation locally.

.. code-block:: bash

   cim docs serve [--port PORT] [--host HOST]    # default: localhost:8000

release
-------

Create release tags across workspace repositories.

.. code-block:: bash

   cim release --tag TAG [--include PATTERNS] [--exclude PATTERNS] [--dry-run]
   cim release --genconfig

``--genconfig`` writes a release configuration (``sdk_release.yml``) instead of
tagging.

config
------

Manage the user configuration file.

.. code-block:: bash

   cim config [--list] [--get KEY] [--path] [--template] [--create [--force]]
              [--edit] [--validate]

See :doc:`/reference/configuration` for configuration file details.

utils
-----

hash-copy-files
~~~~~~~~~~~~~~~

Compute and update SHA256 hashes for ``copy_files`` entries in the workspace
``sdk.yml``.

.. code-block:: bash

   cim utils hash-copy-files [FILE] [--dry-run] [--verbose] [--add-missing]

hash-toolchains
~~~~~~~~~~~~~~~

Compute and update SHA256 hashes for toolchain archives already in the
mirror. Entries are matched by their ``name`` field.

.. code-block:: bash

   cim utils hash-toolchains [TOOLCHAIN] [--dry-run] [--verbose] [--add-missing]

sync-copy-files
~~~~~~~~~~~~~~~

Run ``copy_files`` again to sync files into the workspace.

.. code-block:: bash

   cim utils sync-copy-files [FILE] [--dry-run] [--verbose] [--force]

update
~~~~~~

Update the ``cim`` binary to the latest release.

.. code-block:: bash

   cim utils update

docker
------

Generate a Dockerfile for containerized development (experimental). See
:doc:`/howto/use-docker`.

.. code-block:: bash

   cim docker create --target NAME [--source URL] [--version VERSION]
                     [--distro IMAGE] [--output PATH] [--force]
