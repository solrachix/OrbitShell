# OrbitShell Agent Notes

## Main Lists Policy

This policy applies only to the app's main navigation and content lists for now:

- sidebar search results
- explorer / file tree
- file preview code viewers
- other future primary navigation or content lists with potentially large item counts

## Scrolling

- Prefer native `gpui` scrolling behavior first.
- Do not add custom wheel smoothing, inertia, or manual scroll stepping unless profiling proves a clear need.
- If scrolling feels bad, investigate render cost before changing wheel behavior.

## Rendering

- Prefer lazy or virtualized rendering for long lists.
- Use `uniform_list` when rows are uniform height.
- Use `list` when row heights vary and virtualization is still needed.
- Do not render the full item set for large navigation/content lists if only a visible subset is needed.

## Data Preparation

- Precompute row data outside render when possible.
- Cache expensive per-item transforms that would otherwise run every frame.
- For file previews, avoid re-splitting the full file content on every render.

## Interaction

- Keep selection, keyboard navigation, and click behavior independent from the virtualization strategy.
- Preserve the existing UX when moving a list to lazy rendering.

## Practical Rule

- Small lists can stay as normal `div` trees.
- Main lists that can grow large should default to native scroll plus lazy rendering.
