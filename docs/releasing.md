# Releasing

The release workflow is intentionally manual. It reads the version from `herdr-plugin.toml`, requires
the same version in `Cargo.toml`, and refuses to reuse an existing `v<version>` tag. A successful run
builds four native binaries, generates `SHA256SUMS`, records GitHub build provenance, and creates both
the tag and GitHub release.

## Prepare a version

1. Run `mise run bump -- <version>`, for example `mise run bump -- 0.1.1`.
2. Run the local quality gate listed in the README.
3. Merge the change to the default branch.
4. Run the **Release** workflow from the default branch.

The bump task requires a new `MAJOR.MINOR.PATCH` version. It stages and validates the plugin manifest,
the root Cargo package, and the root package entry in `Cargo.lock`, then replaces each file atomically.
If a replacement fails, it restores the originals from backups.

The workflow will not publish from a private repository or a non-default branch. Do not create the
tag manually; the workflow creates the exact workflow commit's tag and a draft release after all
build and integrity checks pass. The release remains a draft until all five assets have been uploaded
and their names verified.

The workflow removes an incomplete draft and its tag when publishing fails. If cancellation or an API
failure prevents that cleanup, inspect the draft and its assets before retrying:

```sh
gh release view v0.1.0 --json isDraft,assets
```

If the draft is incomplete, delete that draft and its workflow-created tag, then dispatch the same
version again:

```sh
gh release delete v0.1.0 --cleanup-tag
```

## Public-repository checklist

The repository is private while this automation is being prepared, so the public installation and an
actual release workflow dispatch have not yet been exercised. Immediately after making it public:

- [ ] Add repository topics such as `herdr`, `plugin`, `pane`, and `navigation`.
- [ ] Add a ruleset for the default branch requiring **Linux quality gate** and **macOS integration**.
- [ ] Require pull requests and block force pushes or deletion of the default branch.
- [ ] Run the **Release** workflow once and confirm it creates `v0.1.0` and all five assets.
- [ ] Confirm the release has build-provenance attestations for all four binaries.
- [ ] On a machine without Rust, run `herdr plugin install jeiea/herdr-move-pane` with Herdr 0.8 or
  later and invoke at least one action.
- [ ] Reinstall with `--ref v0.1.0` and confirm the pinned-version path works.
