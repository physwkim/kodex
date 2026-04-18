package main

import (
	"fmt"
	"os"
)

type Config struct {
	Name    string
	Version int
}

func NewConfig(name string) *Config {
	return &Config{Name: name, Version: 1}
}

func (c *Config) Display() {
	fmt.Printf("%s v%d\n", c.Name, c.Version)
}

func main() {
	cfg := NewConfig("app")
	cfg.Display()
	os.Exit(0)
}
