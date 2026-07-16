package interop

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/quickfixgo/quickfix"
)

const soh = byte(1)

func TestQuickFIXGoFIXT11FIX50SP2Interop(t *testing.T) {
	binary := os.Getenv("BUNTING_SERVER_BIN")
	if binary == "" {
		t.Fatal("BUNTING_SERVER_BIN must name the built bunting-server binary")
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	endpoint := listener.Addr().String()
	listener.Close()

	config := filepath.Join(t.TempDir(), "server.json")
	contents := fmt.Sprintf(`{
  "version": 1,
  "profile": "local",
  "storage": {"kind":"memory","path":null,"max_runs":4,"max_commands":1000,"max_events_per_run":10000},
  "fix": {"bind":%q,"sender_comp_id":"BUNTING","target_comp_id":"HUMAN","username":"participant","password":"bunting-local-dev","role":"participant","participant_id":1,"run_id":1,"heartbeat_seconds":30,"max_connections":1,"max_message_bytes":16384,"max_journal_messages":512,"max_pending_inbound":64,"tls":{"mode":"disabled"}},
  "admin": null,
  "scenario": null,
  "runtime": null,
  "relay": null
}`, endpoint)
	if err := os.WriteFile(config, []byte(contents), 0o600); err != nil {
		t.Fatal(err)
	}
	command := exec.Command(binary, config)
	var serverLog bytes.Buffer
	command.Stderr = &serverLog
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = command.Process.Kill()
		_ = command.Wait()
	})

	connection := dialBounded(t, endpoint, &serverLog)
	defer connection.Close()
	reader := bufio.NewReader(connection)
	if _, err := connection.Write(outbound("A", 1).Bytes()); err != nil {
		t.Fatal(err)
	}
	logon := parseInbound(t, readFrame(t, reader))
	assertField(t, &logon.Header.FieldMap, 35, "A")
	assertField(t, &logon.Body.FieldMap, 1137, "9")
	assertField(t, &logon.Body.FieldMap, 10000, "bunting.fixlatest.competition.v1")
	assertField(t, &logon.Body.FieldMap, 10004, "participant")

	discovery := outbound("x", 2)
	discovery.Body.SetString(320, "quickfixgo-discovery")
	if _, err := connection.Write(discovery.Bytes()); err != nil {
		t.Fatal(err)
	}
	report := parseInbound(t, readFrame(t, reader))
	assertField(t, &report.Header.FieldMap, 35, "y")
	assertField(t, &report.Body.FieldMap, 10016, "discovery")
	payload, err := report.Body.GetString(10020)
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	if err := json.Unmarshal([]byte(payload), &decoded); err != nil {
		t.Fatalf("invalid Bunting discovery JSON: %v", err)
	}
	if decoded["run_id"] != float64(1) {
		t.Fatalf("unexpected run_id: %v", decoded["run_id"])
	}
}

func outbound(messageType string, sequence int) *quickfix.Message {
	message := quickfix.NewMessage()
	message.Header.SetString(8, "FIXT.1.1")
	message.Header.SetString(35, messageType)
	message.Header.SetString(49, "HUMAN")
	message.Header.SetString(56, "BUNTING")
	message.Header.SetInt(34, sequence)
	message.Header.SetString(52, "20260716-12:00:00")
	if messageType == "A" {
		message.Body.SetInt(98, 0)
		message.Body.SetInt(108, 30)
		message.Body.SetString(1137, "9")
		message.Body.SetString(553, "participant")
		message.Body.SetString(554, "bunting-local-dev")
		message.Body.SetString(10000, "bunting.fixlatest.competition.v1")
		message.Body.SetString(10004, "participant")
	}
	return message
}

func dialBounded(t *testing.T, endpoint string, serverLog *bytes.Buffer) net.Conn {
	t.Helper()
	for range 100 {
		connection, err := net.DialTimeout("tcp", endpoint, 20*time.Millisecond)
		if err == nil {
			return connection
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("server did not bind: %s", serverLog.String())
	return nil
}

func readFrame(t *testing.T, reader *bufio.Reader) []byte {
	t.Helper()
	var frame []byte
	for {
		field, err := reader.ReadBytes(soh)
		if err != nil {
			t.Fatal(err)
		}
		frame = append(frame, field...)
		if strings.HasPrefix(string(field), "10=") {
			return frame
		}
	}
}

func parseInbound(t *testing.T, frame []byte) *quickfix.Message {
	t.Helper()
	message := quickfix.NewMessage()
	if err := quickfix.ParseMessage(message, bytes.NewBuffer(frame)); err != nil {
		t.Fatalf("QuickFIX/Go rejected Bunting frame: %v", err)
	}
	return message
}

func assertField(t *testing.T, fields *quickfix.FieldMap, tag quickfix.Tag, expected string) {
	t.Helper()
	actual, err := fields.GetString(tag)
	if err != nil {
		t.Fatal(err)
	}
	if actual != expected {
		t.Fatalf("tag %d: got %q, want %q", tag, actual, expected)
	}
}
