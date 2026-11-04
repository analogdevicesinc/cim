How to Clone a Subset of Repositories
=====================================

Large manifests often contain repositories that not every user needs, such as
documentation sites, test fixtures or optional components. Repository groups
and name patterns let each user pick what ends up in their workspace.

Assign Groups
-------------

Each entry in ``gits`` can declare one or more groups:

.. code-block:: yaml

   gits:
     - name: app
       url: https://github.com/octocat/Hello-World.git
       commit: master

     - name: docs-site
       url: https://github.com/octocat/Spoon-Knife.git
       commit: main
       group: docs

     - name: extras
       url: https://github.com/octocat/git-consortium.git
       commit: master
       group: [docs, optional]

A repository without ``group`` belongs to the ``default`` group.

Select Groups
-------------

``--include-group`` and ``--exclude-group`` take comma-separated group names:

.. code-block:: bash

   # Everything except the optional repositories (app, docs-site)
   cim init -t my-project --exclude-group optional

   # Only the default and docs groups (app, docs-site, extras)
   cim init -t my-project --include-group default,docs

   # Only the documentation repositories (docs-site, extras)
   cim init -t my-project --include-group docs

The same options work for ``cim update``, ``cim foreach``, ``cim makefile``
and ``cim install pip`` (for per-repository Python dependencies). ``cim init``
stores the selection in the workspace; ``cim update``, ``cim makefile`` and
``cim install pip`` reuse it when you do not pass group options, and
``cim update --all`` clears it.

Select by Name
--------------

``--match`` takes a regular expression on the repository name. Comma-separated
values are alternatives:

.. code-block:: bash

   cim init -t my-project --match "app,docs"
   cim foreach "git status" --match "^app$"

``--match`` can be combined with the group options; a repository must satisfy
both to be included.

Install Steps Tied to Repositories
----------------------------------

An ``install`` step that only makes sense when a repository is present can say
so with ``depends_on_gits``. If that repository is excluded, the step is
dropped from the generated Makefile (and so are steps that depend on it):

.. code-block:: yaml

   install:
     - name: docs-tools
       depends_on_gits:
         - docs-site
       commands:
         - "@echo Installing documentation tools"
