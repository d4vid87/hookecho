// Advanced-mode Pages worker: the only reason it exists is the www redirect. Both hookecho.io and
// www.hookecho.io are custom domains on this project, so without this the site answers on two
// names and gets indexed twice.
// ponytail: a _redirects rule cannot do this — Pages matches those on the path only, hostnames are
// a Netlify feature. Everything that is not www falls straight through to the static assets.
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.hostname.startsWith("www.")) {
      url.hostname = url.hostname.slice(4);
      return Response.redirect(url.toString(), 301);
    }
    return env.ASSETS.fetch(request);
  },
};
