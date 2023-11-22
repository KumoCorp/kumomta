---
hide:
  - toc
  - footer
---
<script type="module" src="https://unpkg.com/rapidoc/dist/rapidoc-min.js"></script>
<style>
#rapidoccontainer {
    height: auto;
}
</style>
<div id="rapidoccontainer"></div>
<script>
// Examine the mkdocs palette component and return the name of
// the rapidoc theme that corresponds to the active palette
function palette_to_theme(palette) {
  if (palette && typeof palette.color === "object") {
    return palette.color.scheme === "slate" ? "dark" : "light";
  }
  return "light";
}

window.addEventListener('DOMContentLoaded', (event) => {
  // Create a rapi-doc element.
  // I can't figure out how to get mkdocs to allow me to define
  // this rapidoc custom html element directly, so we're doing
  // this dynamically once the page loads.
  var doc = document.createElement('rapi-doc')
  // Point it to the kumod spec
  doc.setAttribute('spec-url', "/reference/kumod.openapi.json");
  // Set the theme appropriately
  doc.setAttribute('theme', palette_to_theme(__md_get("__palette")));

  // Some tweaks to make it fit better into our mkdocs
  // <https://rapidocweb.com/api.html>
  doc.setAttribute('show-header', 'false');
  doc.setAttribute('allow-try', 'true');
  doc.setAttribute('allow-server-selection', 'false');
  doc.setAttribute('allow-authentication', 'false');
  doc.setAttribute('allow-spec-file-download', 'true');
  doc.setAttribute('show-curl-before-try', 'true');
  doc.setAttribute('server-url', 'http://127.0.0.1:8000');

  // Now insert this element into the DOM
  document.getElementById('rapidoccontainer').appendChild(doc);

  // Subscribe to component changes, so that we can detect when
  // the user changes light/dark mode
  const ref = document.querySelector("[data-md-component=palette]");
  component$.subscribe(component => {
    if (component.ref === ref) {
      // Update the rapidoc theme to match the mkdoc theme
      doc.setAttribute('theme', palette_to_theme(component));
    }
  })

})
</script>
