const pluginRss = require("@11ty/eleventy-plugin-rss");

module.exports = function(config) {
  config.addPlugin(pluginRss);
  config.addPassthroughCopy("style.css");
};
