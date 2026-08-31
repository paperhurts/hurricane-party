<script lang="ts">
  import { applyTheme } from "../lib/theme";

  // Each window is its own document, so each applies the theme itself. Cheap:
  // it is a handful of custom properties on :root, from design/tokens.json.
  applyTheme();
</script>

<!-- Placeholder interior. v0.4b replaces this with the sprite renderer; the
     geometry and the active/inactive states are what v0.4a is proving. -->
<div class="chrome" data-active="true">
  <div class="titlebar">PLAYLIST</div>
  <div class="body">playlist</div>
</div>
