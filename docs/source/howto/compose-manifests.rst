How to Compose Manifests
========================

When several targets share most of their content, a target can build on top
of another one with ``extends`` instead of duplicating the whole manifest.

Extend a Base Target
--------------------

Given a base target ``targets/base/sdk.yml``:

.. code-block:: yaml

   variables:
     GREETING: Hello

   gits:
     - name: app
       url: https://github.com/octocat/Hello-World.git
       commit: master
       build: |
         @echo "${{ GREETING }} from app"

     - name: tools
       url: https://github.com/octocat/Spoon-Knife.git
       commit: main

   build:
     depends_on:
       - app

A derived target ``targets/derived/sdk.yml`` reuses it:

.. code-block:: yaml

   extends: base

   gits:
     - name: extra
       url: https://github.com/octocat/git-consortium.git
       commit: master

   overlay:
     gits:
       remove:
         - tools
       modify:
         - name: app
           build: |
             @echo "${{ GREETING }} from the derived app"

     variables:
       set:
         GREETING: Hej

``cim init -t derived`` produces a workspace with ``app`` and ``extra`` (not
``tools``), and ``make sdk-build`` prints ``Hej from the derived app``.

``extends`` accepts:

- ``extends: base`` — a target in the same manifest source
- ``extends: base@v2.0`` — pinned to a branch or tag of the manifest
  repository
- a mapping, for a base that lives in another manifest repository:

  .. code-block:: yaml

     extends:
       target: base
       version: v2.0
       source: https://github.com/org/other-manifests

What the Derived Target Can Change
----------------------------------

A derived ``sdk.yml`` carries two kinds of content:

- **New entries**, added directly to the normal ``gits``, ``toolchains``,
  ``install``, ``copy_files`` and ``variables`` sections. A name that collides
  with an inherited entry is an error.
- **overlay** operations against inherited content: ``remove`` (by name, or by
  ``dest`` for ``copy_files``) and ``modify`` (patch the listed fields of an
  inherited entry) for ``gits``, ``toolchains``, ``install`` and
  ``copy_files``; ``set`` and ``remove`` for ``variables``. There is no
  ``add``: new entries go in the normal sections.

For each list, the merge order is fixed: remove (from the base), then combine
(base plus the derived target's new entries), then modify. Removing or
modifying an entry that does not exist is an error, not a silent no-op.

The phase sections (``envsetup``, ``build``, ``test``, ``clean``, ``flash``,
``help``) and ``build_folder``, ``direnv`` and ``phases`` replace the base's
value as a whole. ``makefile_include`` is the exception: entries from the base
and the derived target are combined.

Dependency Files
----------------

``os-dependencies.yml`` and ``python-dependencies.yml`` are not merged. Each
level of the ``extends`` chain may have its own, and ``cim install os-deps``
and ``cim install pip`` install the packages from every level.

``cim init`` copies every level's files into the workspace unchanged. The
requested target's files keep their usual names at the workspace root; each
ancestor's files go to ``.cim/target-overlays/<target>-sdk.yml`` (and
``<target>-os-dependencies.yml``, ``<target>-python-dependencies.yml``).

See the ``overlay-example`` target in
https://github.com/joabech/cim-manifests for a complete example.
