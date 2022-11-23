import {Component, OnInit} from '@angular/core';
import {FormControl, FormGroup} from "@angular/forms";
import {MatSnackBar} from "@angular/material/snack-bar";
import {ServerBridgeService} from "../services/server-bridge.service";
import {Session} from "../../models/session";
import {SessionService} from "../services/session.service";

@Component({
  selector: 'app-login',
  templateUrl: './login.component.html',
  styleUrls: ['./login.component.scss']
})
export class LoginComponent {

  currentSession: Session;

  form = new FormGroup({
    host: new FormControl(''),
    username: new FormControl(''),
    password: new FormControl(''),
  })

  constructor(private snackBar: MatSnackBar, private serverBridge: ServerBridgeService, private sessionService: SessionService) {
    this.sessionService.currentSession$.subscribe(session => this.currentSession = session);
  }

  connect() {
    this.snackBar.open('Connection in progress...', '', {duration: 1000});

    this.serverBridge.connect(this.form.value.username as string, this.form.value.password as string, this.form.value.host as string).subscribe(start_info => {
      if (start_info.websocket_port && start_info.websocket_port > 0) {
        this.snackBar.open('success', '', {duration: 1000});
        this.currentSession.sessionId = start_info.session_id;
        this.currentSession.desktopSize = start_info.initial_desktop_size;
        this.currentSession.active = true;
      } else {
        this.snackBar.open('failure', '', {duration: 1000});
      }
    })
  }
}
