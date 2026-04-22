package tree_sitter_beer_test

import (
	"testing"

	tree_sitter "github.com/smacker/go-tree-sitter"
	"github.com/tree-sitter/tree-sitter-beer"
)

func TestCanLoadGrammar(t *testing.T) {
	language := tree_sitter.NewLanguage(tree_sitter_beer.Language())
	if language == nil {
		t.Errorf("Error loading Beer grammar")
	}
}
