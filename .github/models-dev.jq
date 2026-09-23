# Flattens models.dev's api.json into the TSV `pricing::tests::matches_models_dev`
# reads: one row per model aiburn should price plus a `-fast` row per published
# Fast mode, as provider, model, input, output, cache read, cache write, then
# the long-context tier's size, input, output and cache read (empty if none).
# Only tool-calling, text-only models: embeddings, image and audio models never
# show up in agent logs. OpenAI models older than Codex (2025-04-16) can't be in
# its logs either.
[.anthropic, .openai][] as $p
| $p.models[]
| select(.tool_call and .modalities.output == ["text"])
| select($p.id == "anthropic" or .release_date >= "2025-04-16")
| .id as $id
| (.cost.tiers // [] | map(select(.tier.type == "context")) | first) as $lc
| ([$p.id, $id, .cost.input, .cost.output, .cost.cache_read, .cost.cache_write,
    $lc.tier.size, $lc.input, $lc.output, $lc.cache_read]),
  (.experimental.modes.fast.cost // empty
    | [$p.id, "\($id)-fast", .input, .output, .cache_read, .cache_write,
       null, null, null, null])
| @tsv
