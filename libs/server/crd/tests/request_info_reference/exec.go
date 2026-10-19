package main

// Generates exec_query_reference.json (RFC 0004): how the apiserver reads an
// exec or attach query into PodExecOptions: URL.Query(), then what the
// generated Convert_url_Values_To_v1_PodExecOptions (k8s.io/kubernetes) does,
// field by field, with the same apimachinery helpers. The api crate's
// parse_exec_query test replays it.

import (
	"encoding/json"
	"net/url"
	"os"

	"k8s.io/apimachinery/pkg/runtime"
)

type execResult struct {
	Query     string   `json:"query"`
	Command   []string `json:"command"`
	Container string   `json:"container"`
	Stdin     bool     `json:"stdin"`
	Stdout    bool     `json:"stdout"`
	Stderr    bool     `json:"stderr"`
	TTY       bool     `json:"tty"`
}

// readBool is what the generated Convert_url_Values_To_v1_PodExecOptions does
// for each boolean field.
func readBool(values url.Values, key string) bool {
	var out bool
	if v, ok := values[key]; ok && len(v) > 0 {
		if err := runtime.Convert_Slice_string_To_bool(&v, &out, nil); err != nil {
			panic(err)
		}
	}
	return out
}

func writeExecReference() {
	raw, err := os.ReadFile("exec_cases.json")
	if err != nil {
		panic(err)
	}
	var queries []string
	if err := json.Unmarshal(raw, &queries); err != nil {
		panic(err)
	}
	out := make([]execResult, 0, len(queries))
	for _, query := range queries {
		// What req.URL.Query() does: a malformed pair is dropped, not fatal.
		values, _ := url.ParseQuery(query)
		r := execResult{Query: query, Command: []string{}}
		if v, ok := values["command"]; ok {
			r.Command = v
		}
		if v, ok := values["container"]; ok && len(v) > 0 {
			if err := runtime.Convert_Slice_string_To_string(&v, &r.Container, nil); err != nil {
				panic(err)
			}
		}
		r.Stdin = readBool(values, "stdin")
		r.Stdout = readBool(values, "stdout")
		r.Stderr = readBool(values, "stderr")
		r.TTY = readBool(values, "tty")
		out = append(out, r)
	}
	encoded, _ := json.MarshalIndent(out, "", "  ")
	if err := os.WriteFile("../exec_query_reference.json", append(encoded, '\n'), 0o644); err != nil {
		panic(err)
	}
}
