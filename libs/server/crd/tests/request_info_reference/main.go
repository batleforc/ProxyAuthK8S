// Generates request_info_reference.json: the verb, resource and name the real
// apiserver RequestInfoFactory computes for each case of cases.json, and
// exec_query_reference.json (exec.go). Run by
// `task gen:request-info-reference` (needs Go and network), never in CI; the
// output is versioned and checked by crd's request_info_reference test.
package main

import (
	"encoding/json"
	"net/http"
	"os"

	"k8s.io/apimachinery/pkg/util/sets"
	"k8s.io/apiserver/pkg/endpoints/request"
)

type testCase struct {
	Method string `json:"method"`
	Path   string `json:"path"`
	Query  string `json:"query"`
}

type result struct {
	testCase
	Error       bool   `json:"error"`
	IsResource  bool   `json:"is_resource_request"`
	Verb        string `json:"verb"`
	APIGroup    string `json:"api_group"`
	Version     string `json:"version"`
	Namespace   string `json:"namespace"`
	Resource    string `json:"resource"`
	Name        string `json:"name"`
	Subresource string `json:"subresource"`
}

func main() {
	raw, err := os.ReadFile("cases.json")
	if err != nil {
		panic(err)
	}
	var cases []testCase
	if err := json.Unmarshal(raw, &cases); err != nil {
		panic(err)
	}
	factory := request.RequestInfoFactory{
		APIPrefixes:          sets.NewString("api", "apis"),
		GrouplessAPIPrefixes: sets.NewString("api"),
	}
	out := make([]result, 0, len(cases))
	for _, c := range cases {
		target := c.Path
		if c.Query != "" {
			target += "?" + c.Query
		}
		req, err := http.NewRequest(c.Method, "https://apiserver"+target, nil)
		if err != nil {
			out = append(out, result{testCase: c, Error: true})
			continue
		}
		info, err := factory.NewRequestInfo(req)
		r := result{testCase: c, Error: err != nil}
		if err == nil {
			r.IsResource = info.IsResourceRequest
			r.Verb = info.Verb
			r.APIGroup = info.APIGroup
			r.Version = info.APIVersion
			r.Namespace = info.Namespace
			r.Resource = info.Resource
			r.Name = info.Name
			r.Subresource = info.Subresource
		}
		out = append(out, r)
	}
	encoded, _ := json.MarshalIndent(out, "", "  ")
	if err := os.WriteFile("../request_info_reference.json", append(encoded, '\n'), 0o644); err != nil {
		panic(err)
	}
	writeExecReference()
}
