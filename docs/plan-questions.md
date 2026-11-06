# Questions for docs/milestones/01-native-slice.md

1. ASK — Review budget. The write-plan loop allows three final DRY-RUN passes; the third
   reported 14 blockers, all fixed in the plan (Review history, "Final dry-run 3"), but no
   critic has verified the fixed version. Options: (a) run one more DRY-RUN pass and mark
   the plan final only if it reports no blockers; (b) accept the plan as final now and let
   the CP reviewers catch remaining gaps. Recommendation: (a); every pass so far found
   real gaps, though each was smaller than the last.
2. ASK — CI and toolchain pinning. Options: (a) CP1 adds `rust-toolchain.toml` (1.99.0) and
   a Linux-only GitHub Actions job running `cargo test` and both clippy invocations; this
   needs a GitHub remote (the repository has none) and binutils on the runner; (b) local
   checks only. Recommendation: (a) once a remote exists; the previous project's CI went
   red for a week unnoticed, so the job should be checked after every push.
