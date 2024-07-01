FROM docker.io/squidfunk/mkdocs-material
RUN pip install mkdocs-macros-plugin mkdocs-include-markdown-plugin mkdocs-exclude mkdocs-git-revision-date-localized-plugin
