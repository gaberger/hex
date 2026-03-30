package main

import (
	"testing"
	"net/http"
	"net/http/httptest"
)

func TestHandleRequest(t *testing.T) {
	type args struct {
		req *http.Request
	}
	tests := []struct {
		name    string
		args    args
		want    string
		wantErr bool
	}{
		{
			name: "happy path",
			args: args{
				req: httptest.NewRequest("GET", "/items", nil),
			},
			want:    "OK",
			wantErr: false,
		},
		{
			name: "empty path",
			args: args{
				req: httptest.NewRequest("GET", "", nil),
			},
			want:    "Not Found",
			wantErr: false,
		},
		{
			name: "invalid method",
			args: args{
				req: httptest.NewRequest("POST", "/items", nil),
			},
			want:    "Method Not Allowed",
			wantErr: false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			rr := httptest.NewRecorder()
			handleRequest(rr, tt.args.req)
			if rr.Code != http.StatusOK {
				t.Errorf("handleRequest() = %v, want %v", rr.Code, http.StatusOK)
			}
			if rr.Body.String() != tt.want {
				t.Errorf("handleRequest() = %v, want %v", rr.Body.String(), tt.want)
			}
		})
	}
}