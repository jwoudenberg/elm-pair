---
title: Home
layout: root.html
---

Elm-pair helps you write Elm code. You tell Elm-pair about the change you want to make and it will do the actual work. It's a bit like using an IDE except you don't need to learn any keyboard shortcuts.

You talk to Elm-pair by making a change in your code and saving it. Elm-pair will notice the change you made, and if it understands your intent will respond with a change off its own.

Elm-pair is in early development. The short video below demonstrates current functionality.

<div style="padding:41.87% 0 0 0;position:relative;"><iframe src="https://player.vimeo.com/video/662666351?h=096340b0bc" style="position:absolute;top:0;left:0;width:100%;height:100%;" frameborder="0" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen></iframe></div><script src="https://player.vimeo.com/api/player.js"></script>

<h2 id="news">News</h2>

{% for news in collections.news %}
### {{news.data.title}}

{{news.templateContent }}
{% endfor %}
