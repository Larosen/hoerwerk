"""Established non-fiction authors: their books may appear in "Für dich" (see discover.py).

Who is on it: authors who published widely noticed non-fiction in the last ten years that reflects the current
state of research or knowledge – scientists and physicians writing for the public, historians, economists,
science journalists, and public figures with serious books on politics and society. Kept off on purpose:
authors whose work the field regards as pseudo-science or who were caught out by plagiarism or fabrication.

This is a list of reputations, not a check of every book. Besides it, discover.py also accepts every author with
a Perlentaucher author page (books reviewed in the arts pages) and every author named in the review feeds, and
the admin can add names in the settings.
"""
from __future__ import annotations

TRUSTED_AUTHORS = (
    # -- named by the owner --------------------------------------------------------------------------
    "Yuval Noah Harari", "Stefanie Stahl", "Robert Habeck", "Richard David Precht", "Matthias Brandt",
    "Constantin Schreiber", "Rüdiger Maas",

    # -- science for everyone (German-speaking) -----------------------------------------------------
    "Mai Thi Nguyen-Kim", "Harald Lesch", "Ranga Yogeshwar", "Florian Freistetter", "Josef M. Gaßner",
    "Heino Falcke", "Ulrich Walter", "Ernst Peter Fischer", "Stefan Klein", "Bas Kast", "Henning Beck",
    "Martin Korte", "Volker Busch", "Leon Windscheid", "Lars Jaeger", "Sabine Hossenfelder", "Manuela Lenzen",
    "Katharina Zweig", "Dirk Steffens", "Frank Schätzing", "Jens Soentgen",

    # -- medicine and health ------------------------------------------------------------------------------
    "Giulia Enders", "Eckart von Hirschhausen", "Yael Adler", "Andreas Michalsen", "Michael Tsokos",
    "Christian Drosten", "Siddhartha Mukherjee", "Atul Gawande", "Matthew Walker",
    "Lisa Sanders", "Bessel van der Kolk", "Tim Spector", "Ed Yong",

    # -- psychology, mind and behaviour -------------------------------------------------------------------
    "Daniel Kahneman", "Robert Sapolsky", "Lisa Feldman Barrett", "Steven Pinker", "Jonathan Haidt",
    "Adam Grant", "Angela Duckworth", "Brené Brown", "Susan Cain", "Charles Duhigg", "Cal Newport",
    "James Clear", "Ethan Kross", "Daniel H. Pink", "Esther Perel", "Julia Shaw",
    "Svenja Flaßpöhler", "Jesper Juul", "Nicola Schmidt", "Herbert Renz-Polster", "Remo Largo",

    # -- climate, nature and environment --------------------------------------------------------------------
    "Stefan Rahmstorf", "Hans Joachim Schellnhuber", "Claudia Kemfert", "Mojib Latif", "Maja Göpel",
    "Kate Raworth", "Elizabeth Kolbert", "David Wallace-Wells", "Bill Gates", "Merlin Sheldrake", "Sara Schurmann", "Bernhard Pötter", "Christian Stöcker",
    "Johan Rockström", "Hannah Ritchie", "Hans Rosling", "Anna Rosling Rönnlund",

    # -- history ----------------------------------------------------------------------------------------------
    "Christopher Clark", "Heinrich August Winkler", "Herfried Münkler", "Karl Schlögel", "Volker Ullrich",
    "Andreas Kossert", "Ilko-Sascha Kowalczuk", "Harald Jähner", "Florian Illies", "Philipp Blom",
    "Götz Aly", "Norbert Frei", "Ulrich Herbert", "Peter Longerich", "Michael Wildt", "Frank Bösch",
    "Andreas Wirsching", "Johannes Fried", "Mischa Meier", "Tom Holland", "Mary Beard", "Peter Frankopan",
    "Timothy Snyder", "Anne Applebaum", "Ian Kershaw", "Antony Beevor", "Margaret MacMillan",
    "Simon Sebag Montefiore", "Serhii Plokhy", "Orlando Figes", "Jared Diamond", "David Graeber",
    "David Wengrow", "Joseph Henrich", "Rutger Bregman", "Johannes Krause", "Felix Bohr",

    # -- politics, society and philosophy -----------------------------------------------------------------
    "Andreas Reckwitz", "Armin Nassehi", "Steffen Mau", "Hartmut Rosa", "Jürgen Habermas", "Navid Kermani",
    "Carolin Emcke", "Heinz Bude", "Nicole Deitelhoff", "Carlo Masala", "Sönke Neitzel", "Gerd Koenen",
    "Katja Gloger", "Masha Gessen", "Svetlana Alexievich", "Heribert Prantl", "Sascha Lobo",
    "Marina Weisband", "Kübra Gümüşay", "Alice Hasters", "Natascha Strobl", "Daniel Ziblatt",
    "Steven Levitsky", "Francis Fukuyama", "Michael J. Sandel", "Kwame Anthony Appiah", "Martha Nussbaum",
    "Wolfram Eilenberger", "Markus Gabriel", "Ivan Krastev", "Thomas Ramge",
    "Jagoda Marinić", "Ferda Ataman", "Aladin El-Mafaalani", "Naika Foroutan", "Mohamed Amjahid",
    "Michel Friedman", "Ronen Steinke", "Georg Mascolo", "Bastian Berbner",

    # -- economy and work -----------------------------------------------------------------------------------
    "Thomas Piketty", "Mariana Mazzucato", "Marcel Fratzscher", "Isabella Weber", "Ulrike Herrmann",
    "Daron Acemoglu", "James A. Robinson", "Esther Duflo", "Abhijit V. Banerjee", "Joseph E. Stiglitz",
    "Clemens Fuest", "Jens Südekum", "Monika Schnitzer", "Veronika Grimm",
    "Rüdiger Bachmann", "Philippa Sigl-Glöckner", "Tim Harford", "Shoshana Zuboff",

    # -- technology and the future ------------------------------------------------------------------------------
    "Max Tegmark", "Stuart Russell", "Kate Crawford", "Mustafa Suleyman", "Melanie Mitchell", "Miriam Meckel", "Ranga Yogeshwar",

    # -- physics and space ---------------------------------------------------------------------------------------
    "Carlo Rovelli", "Brian Greene", "Neil deGrasse Tyson", "Sean Carroll", "Lisa Randall", "Marcus Chown",
    "Kip Thorne", "Lisa Kaltenegger",

    # -- lives and memoirs with weight ----------------------------------------------------------------------------
    "Angela Merkel", "Barack Obama", "Michelle Obama", "Joschka Fischer", "Wolfgang Schäuble",
    "Gregor Gysi", "Hape Kerkeling", "Navid Kermani", "Daniel Schreiber", "Alexander Osang",
)

TRUSTED_AUTHORS = tuple(dict.fromkeys(TRUSTED_AUTHORS))      # a name listed twice counts once
